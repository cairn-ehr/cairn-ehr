# ADR-0002 — In-database Rust (pgrx) as the projection performance escape hatch

- **Status:** Accepted
- **Date:** 2026-06-14
- **Refines:** [ADR-0001](0001-fat-postgres-thin-daemon.md)

## Context

[ADR-0001](0001-fat-postgres-thin-daemon.md) places the projections and identity algebra in
Postgres (PL/pgSQL, trigger-maintained) and named a per-projection **escape hatch** to "the Rust
core" if PL/pgSQL proved too slow on Pi-class hardware. That framing carried a tension: relocating
a projection to the **external** Rust sync daemon would (a) put merge logic in the daemon, which
ADR-0001 says carries none, and (b) move logic *out of the database*, losing the unbypassability
and "next to the data" properties that were the whole point, and crossing the
[§9.3](../language-substrate.md#93-integration-boundary) database boundary.

Two things resolve the tension:

1. **pgrx** (Rust framework for PostgreSQL extensions; MIT/Apache-2.0) lets us write Postgres
   functions in **Rust that run inside the database**, callable from SQL and triggers exactly like
   PL/pgSQL. "Rust" and "in-database" stop being two separate buckets — they become one:
   *in-database Rust*.
2. **Deployment reality.** A Pi-class node serves at most a handful of workstations with little
   concurrent access (the [§8](../deployment.md) rural-clinic / off-grid profile; a *busy ED* runs
   on a department server, not a Pi). So the performance risk is **single-operation latency** on a
   weak ARM CPU and SD/USB storage — not throughput or lock contention.

## Decision

Reframe the escape hatch as an **in-database escalation ladder** that never leaves Postgres:

1. **PL/pgSQL** — the default. Most reviewer-legible for set-oriented projection logic; no build
   step.
2. **Rust via pgrx (in-database)** — when a function is hot or algorithmically complex (the
   identity connected-component over a large link graph is the prime candidate). Compiled-Rust
   performance, type-safety, and exhaustive matching, while the function **stays a Postgres
   function**: next to the data, unbypassable, invoked by the same triggers/constraints, inside the
   [§9.3](../language-substrate.md#93-integration-boundary) database boundary.
3. **External Rust** — only if logic genuinely cannot be a database function. Not expected for
   projections.

The thin sync daemon still carries **no** merge logic. "Rust" in the safety bucket
([§9.1](../language-substrate.md#91-selection-rule-by-defect-blast-radius)) now spans both the
external daemon *and* in-database pgrx functions.

## Consequences

- The escape hatch no longer compromises ADR-0001's virtues — it **strengthens** them: Rust speed
  without leaving the database; logic stays unbypassable and next to the data.
- **Reviewer-legibility maps to logic shape:** PL/pgSQL for simple set operations, Rust for
  algorithms. Both score high on [§9.2](../language-substrate.md#92-primary-quality-metric-reviewer-legibility);
  the audited surface stays small and bounded (the set of pgrx functions).
- **Crash surface:** pgrx functions run in-process, but Rust eliminates the memory-unsafety class
  and pgrx converts panics into Postgres errors (transaction abort), not backend crashes.
- **Packaging:** pgrx compiles a native extension per **(architecture, PostgreSQL major version)**.
  The node image must ship the extension built for its arch — ARM64 for the Pi, x86_64 for servers.
  This fits the single-image-per-node / zero-DBA target ([§8](../deployment.md)) but adds a
  per-arch build step to the packaging pipeline. **To confirm:** pgrx support for PostgreSQL 18 (the
  project's floor).
- **The Pi benchmark (the ADR-0001 go/no-go) is re-scoped:** target the rural-clinic profile at
  realistic *low* concurrency and measure **single-operation latency** (one clinician's chart
  read/write), since contention is negligible at Pi scale. Busy-ED volumes belong to the
  department-server profile, which is not performance-constrained. The benchmark now has a designed
  mitigation *before* the external-Rust last resort: rewrite the hot function in pgrx, in place.
