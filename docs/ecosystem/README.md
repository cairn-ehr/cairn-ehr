# Ecosystem evaluations

This directory holds Cairn's **ecosystem evaluations** — design analyses that weigh *external* infrastructure
(candidate plugins, adjacent tools, optional extensions) against the *decided* architecture (the numbered spec
§1–§11 and the [Decision log](../spec/decisions/README.md)).

An ecosystem evaluation is **not architecture and not a decision**. It is the *why of fit*: what an external project
is, where it lands in the [four-layer model](../spec/decisions/0021-layering-the-node-api-and-ui-pluralism.md)
(L0 wire core / L1 enforcement floor / L2 policy+API / L3 UI), which existing primitives it composes with, and what
would have to be true for it to attach **without compromising the safety floor or inter-node compatibility**. Every
subject here is an *optional* extension: a bare Cairn node is a complete EHR without it.

## Why a separate area

The spec stays a clean statement of *what Cairn is*; the [ADR log](../spec/decisions/README.md) stays a clean
statement of *why Cairn decided what it did*. An evaluation of someone else's software is a third thing — *what we
looked at, and what we concluded about fit* — and belongs in neither. When an evaluation surfaces a genuine
architectural commitment, that commitment is ratified separately as an ADR; the evaluation only flags it.

## Index

| Eval | Title | Subjects | Outcome |
|---|---|---|---|
| [0001](0001-agent-and-messaging-plugins-kastellan-localmail.md) | Agent & messaging plugins | [kastellan](https://github.com/hherb/kastellan) · [localmail](https://github.com/hherb/localmail) | Both fit as optional L2/L3 extensions (three-membrane model); spec unchanged at v0.26; one [ADR-0011](../spec/decisions/0011-actor-registry-version-pinning-and-key-custody.md) refinement parked (skill-epoch as a pinned actor determinant) |
| [0003](0003-reference-data-sourcing-medicines-and-terminologies.md) | External reference-data sourcing | WHO INN · RxNorm · DailyMed/openFDA · TGA ARTG · PBS · ICD-10/11 · ICPC · Mondo/ORDO | **Evaluation.** Medicines: INN-anchored substance UUID + public-domain/CC-BY government feeds bundle freely; ATC/DDD, DrugBank, AMH/eTG, SNOMED-AMT excluded; DDI gap real. Disease/injury: **ICD-11 ratified as canonical interlingua → [ADR-0025](../spec/decisions/0025-icd-11-canonical-interlingua-and-local-terminology-overlay.md)** (verbatim codes, offline container, local-terminology overlay); ICPC-3 conditional on its CC variant; Mondo/ORDO (CC BY 4.0) clean substrate; SNOMED/ICD-10-AM/ICPC-2-PLUS excluded. Human-verify items remain. |
| [0004](0004-reference-ui-framework-iced-vs-tauri.md) | Reference-UI framework (the L3 desktop client) | [iced](https://github.com/iced-rs/iced) (MIT, Rust-native) · [Tauri 2](https://github.com/tauri-apps/tauri) (MIT/Apache-2.0, webview) | **Evaluation.** Both AGPL-compatible. **iced** is the more mission-aligned L3 citizen (supply-chain auditability, single-language reviewer-legibility, self-contained wgpu/tiny-skia rendering for the fractal fleet, a stateful Elm model that rhymes with the event/projection record) — recommended **conditional** on [Spike 0004](../spikes/0004-iced-reference-ui-viability.md) retiring two EHR bars (accessibility, complex-text/IME) + Pi latency. L3 is plural → a FAIL tips to a webview/Tauri L3, not a defeat. **Spec unchanged at v0.40; no ADR** (UI tech is below the compatibility boundary). |
