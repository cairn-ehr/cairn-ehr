# 8. Deployment Profiles

| Profile | Hardware floor | Stack |
|---|---|---|
| Solo practice | 1× mini-PC + workstations | Full Postgres each machine; practice node = parent |
| Rural clinic (off-grid) | Raspberry Pi 5 class, solar | Postgres on Pi; sneakernet/3G sync to district |
| Hospital department | 1 small server | Postgres + sync service, scoped mirror |
| Hospital core | HA Postgres pair | Patroni-style failover; parent for departments |
| Regional/national | Cluster | Aggregation, registries, cross-facility matching, master patient index |

## 8.1 Expected population per tier — the figure every latency budget is judged against

A performance budget is unjudgeable without a population to judge it at: a measurement at an
arbitrary size can be quoted as proof of anything. These are the **expected patient populations per
tier**, maintainer-set (2026-09-21, #637), and they are what a §1.2 time budget such as *"5 s to
find an existing chart"* ([§5.11](identity.md), `db/046_patient_search.sql`) is measured against.

| Profile | Expected patients |
|---|---|
| Rural clinic (off-grid), Pi-class | **~50,000** — a mid-sized cluster of small communities around a single medical outpost, with deliberate headroom above the ~25,000 such a cluster actually implies |

The Pi-class figure is the load-bearing one, because Pi-class is the **performance floor**: it is the
weakest tier that must still survive a full partition alone ([§8 topology](topology.md)), and it is
the deployment where a clinician has no faster alternative to fall back on. A workflow that misses
its budget on a hospital node is slow; one that misses it on a Pi-class node in a remote clinic is
the failure paper-parity exists to prevent.

The remaining tiers are deliberately unstated rather than guessed. Add one when a budget needs it,
with the same reasoning attached, rather than inventing a ladder nobody has measured.

Packaging: single container image / Debian package per node; configuration declares tier, parent, sync scope. Zero-DBA target for lower tiers. Where in-database Rust (pgrx, [§9.4](language-substrate.md#94-merge-projection-boundary-fat-postgres-thin-rust-daemon) / [ADR-0002](decisions/0002-in-database-rust-pgrx-escape-hatch.md)) is used, the node image ships the native extension built for its **architecture** (ARM64 for Pi, x86_64 for servers) and PostgreSQL major version — a per-arch build step in the pipeline, transparent at deploy time.
