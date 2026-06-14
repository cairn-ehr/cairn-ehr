# Decision Log (ADRs)

This directory holds Cairn's **Architecture Decision Records**. Each ADR captures *why* a decision
was made — the context, the choice, and its consequences. Aspect documents in
[../](../index.md) describe *what* the architecture is; ADRs explain *why* it is that way.

## Rules

- **Append-only and immutable.** An ADR is never edited to reverse it. To change a decision, write
  a **new** ADR that supersedes the old one; mark the old one `Status: Superseded by ADR-NNNN` and
  the new one `Supersedes: ADR-NNNN`. This is the project's own *"never erase, always overlay"*
  ([identity §5.1](../identity.md#51-linkage-layer-never-merge-always-link)) applied to its
  documentation. (Typo/clarity fixes that don't change meaning are fine.)
- **Numbered and dated.** `NNNN-short-slug.md`, zero-padded, allocated in order.
- **Read before reopening.** Before reopening a settled question, read its ADR — the rationale is
  there on purpose.
- The spec carries **no in-file changelogs**; git is the line history and ADRs are the rationale.

## Index

| ADR | Title | Status | Date |
|---|---|---|---|
| [0000](0000-pre-adr-changelog-v0.1-v0.6.md) | Pre-ADR changelog history (spec v0.1 → v0.6) | Imported (historical) | 2026-06-13 |
| [0001](0001-fat-postgres-thin-daemon.md) | Postgres-intelligence cluster: fat Postgres, thin Rust daemon | Accepted | 2026-06-13 |
| [0002](0002-in-database-rust-pgrx-escape-hatch.md) | In-database Rust (pgrx) as the projection escape hatch | Accepted (refines 0001) | 2026-06-14 |
| [0003](0003-bitemporal-time-and-acknowledged-uncertainty.md) | Bitemporal event time and acknowledged uncertainty | Accepted | 2026-06-14 |
| [0004](0004-dynamic-sync-scope-prefetch-not-authority.md) | Dynamic sync scope: a prefetch hint, not an authority | Accepted | 2026-06-14 |
| [0005](0005-erasure-key-custody-and-crypto-shredding.md) | Erasure as key-custody redistribution: crypto-shredding and a policy-neutral severity ladder | Accepted | 2026-06-14 |

## Template

```markdown
# ADR-NNNN — <title>

- **Status:** Proposed | Accepted | Superseded by ADR-NNNN
- **Date:** YYYY-MM-DD
- **Supersedes:** ADR-NNNN (if any)

## Context
<the forces at play: the problem, constraints, and what made this a real decision.>

## Decision
<the choice, stated plainly.>

## Consequences
<what becomes easier, what becomes harder, what we are now betting on, and how we'd know if the
bet fails.>
```
