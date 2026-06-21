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
| [0006](0006-visibility-scope-replication-and-the-safety-projection.md) | Visibility-scope vs. sync-scope: replication is not the confidentiality boundary; the safety projection and graded sensitivity | Accepted | 2026-06-14 |
| [0007](0007-authorship-and-accountability.md) | Authorship is compositional; accountability is separable | Accepted | 2026-06-15 |
| [0008](0008-point-of-care-identity-possession-and-salvage.md) | Point-of-care identity: possession binding, fast authentication, and work-salvage | Accepted | 2026-06-15 |
| [0009](0009-notification-economy-salience-routing-and-the-acknowledgment-floor.md) | The notification economy: unbundling priority, responsibility-routing, and the acknowledgment floor | Accepted | 2026-06-15 |
| [0010](0010-additive-vs-suppressing-classification.md) | Additive-vs-suppressing classification: structural derivation, the demotion boundary, and automation-complacency detection | Accepted (refines 0007) | 2026-06-15 |
| [0011](0011-actor-registry-version-pinning-and-key-custody.md) | The actor registry: version-pinned immutable identity, behavioral-configuration granularity, and key custody | Accepted (refines 0007) | 2026-06-15 |
| [0012](0012-schema-evolution-event-format-and-legibility-across-time.md) | Schema evolution, event-format migration across the offline fleet, and legibility across time | Accepted | 2026-06-15 |
| [0013](0013-attachments-content-addressed-lazy-blob-tier.md) | Attachments: content-addressed blobs, the lazy byte tier, and reference-eager replication | Accepted | 2026-06-15 |
| [0014](0014-locale-pluggable-matcher-comparators.md) | Locale-pluggable matcher comparators: content-addressed profiles that travel with the data | Accepted | 2026-06-16 |
| [0015](0015-event-serialization-signatures-and-content-addressing.md) | Event serialization, signatures, and content addressing: tagged, migratable primitives over three structural moves | Accepted | 2026-06-16 |
| [0016](0016-record-discovery-and-the-replicated-essential-tier.md) | National-scale record discovery: the replicated essential-state tier and federation admission | Accepted | 2026-06-16 |
| [0017](0017-federation-admission-sovereignty-peering-and-trust-anchors.md) | Federation admission: the sovereignty floor, mutual peering, pluggable trust anchors, and the custodian contract | Accepted | 2026-06-16 |
| [0018](0018-federation-revocation-cascade-and-the-anchor-as-power.md) | Federation revocation: counterparty enforcement, cascade, and the anchor as a position of power | Accepted (refines 0017) | 2026-06-16 |
| [0019](0019-author-scoped-record-export-the-medico-legal-copy.md) | Author-scoped record export: the clinician's medico-legal copy | Accepted (refines 0007) | 2026-06-16 |
| [0020](0020-active-write-thin-encounters-and-the-delete-vs-erase-distinction.md) | The active-write model: thin encounters, type-through authoring, and the delete-vs-erase distinction | Accepted | 2026-06-17 |
| [0021](0021-layering-the-node-api-and-ui-pluralism.md) | Layering, the node API, and UI pluralism: uniform core, plural edges | Accepted | 2026-06-17 |
| [0022](0022-validated-submit-surface-the-write-path.md) | The validated submit surface: the node's write path | Accepted (refines 0021) | 2026-06-17 |
| [0023](0023-native-api-contract-capability-and-conformance.md) | The native API contract: capability description and executable conformance | Accepted (refines 0021) | 2026-06-17 |
| [0024](0024-hard-policy-expression-the-policy-assertion-stream.md) | Hard policy expression: the policy-assertion stream and the effective-policy projection | Accepted (refines 0021) | 2026-06-17 |
| [0025](0025-icd-11-canonical-interlingua-and-local-terminology-overlay.md) | ICD-11 as the canonical classification interlingua and the local-terminology overlay | Accepted | 2026-06-19 |
| [0026](0026-node-durability-and-disaster-recovery.md) | Node durability and disaster recovery: backup-as-cold-peer, new-identity restore, and shred-aware backups | Accepted (refines 0005) | 2026-06-20 |
| [0027](0027-trusted-time-anchoring.md) | Trusted-time anchoring: the clock-confidence grade, the bracketed `t_recorded`, and the pluggable multi-anchor | Accepted (refines 0003) | 2026-06-20 |
| [0028](0028-finalized-closed-contributor-role-enum.md) | The finalized closed contributor-role enum | Accepted (refines 0007) | 2026-06-20 |
| [0029](0029-skill-epoch-as-pinned-actor-determinant.md) | Skill-epoch (and served-model digest) as a pinned determinant of an agent actor's identity | Accepted (refines 0011) | 2026-06-21 |
| [0030](0030-advisory-actor-integration-contract.md) | The advisory-actor integration contract: L2/L3 attachment and authorship through the in-DB floor | Accepted (refines 0021) | 2026-06-21 |

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
