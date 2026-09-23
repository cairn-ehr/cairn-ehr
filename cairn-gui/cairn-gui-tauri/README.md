# `cairn-gui-tauri` — the reference window

Cairn's first runnable clinical surface ([#288](https://github.com/cairn-ehr/cairn-ehr/issues/288)): a
Tauri 2 window whose **front door is the §5.3/§5.8 search-before-create funnel** (slice 2c) — browse for
an existing chart, or register a new patient only if nothing fits — opening onto **one patient's
medication chart** under a persistent identity header, with whole-list sign-off as one human gesture and
per-row cease.

```bash
# Fixture mode — no database, no clinical writes (registering adds to an in-memory population).
cargo run -p cairn-gui-tauri -- --mock
# ...straight onto the sample medication chart (what the accessibility pass uses).
cargo run -p cairn-gui-tauri -- --mock --patient 00000000-0000-0000-0000-000000000001

# Against a real node: the front door, or straight onto one chart with --patient.
cargo run --release -p cairn-gui-tauri -- \
    --conn "$CONN" --key node.key --attester-key dr-a.key [--patient <uuid>]
```

At launch against a real node the window asks whether this node's key may write (all four
`ActorStanding` answers). A "no" does not stop it opening, since reading needs no actor; the line above
both surfaces (the front door and the chart) says what an operator must do instead.

## Where the decisions live

| Question | Answer, and where |
|---|---|
| What does a sign-off gesture attest? | `cairn_medication_view::sign_off_targets` — **one** definition, shared with the node's orchestrator |
| What does the window display? | `cairn_gui_tab_medications::build_view` — every clinical display decision, in Rust, under `cargo test` |
| What does the webview decide? | **Nothing.** It renders `MedListView` and the front door's payloads, and calls commands |
| When does the machine search unasked; how many candidates may the prompt show; what does a registration attest? | `cairn-gui-funnel` (`trigger`, `prompt`, `token`, `session`) |
| What does the clerk read after each failure? | `src/funnel/view.rs` — every sentence, and whether to offer a retry |
| Why is a defect on one line not fatal? | [ADR-0060](../../docs/spec/decisions/0060-partial-validity-a-defect-on-one-line-never-invalidates-another.md) |

## Three things that will surprise a newcomer

1. **No npm, no bundler, no `node_modules`.** `withGlobalTauri` puts `invoke` on `window`, so
   `src-ui/main.js` and `src-ui/funnel.js` *are* the frontend — hand-written, unminified, and exactly
   what runs. The cost is no type checking, paid for by two tests (`src/commands.rs` for `main.js`,
   `src/funnel/commands.rs` for `funnel.js`) that scan for every field access and assert the backend
   actually sends it. Both are verified to fail on a renamed field. Do not "improve" the
   frontend by adding a build step without reading
   [#332](https://github.com/cairn-ehr/cairn-ehr/issues/332) first.

2. **`gen/` is generated and gitignored**; `capabilities/default.json` grants `core:default` and nothing
   else. Every action goes through a command in `src/commands.rs` or `src/funnel/commands.rs`, so the webview needs no
   filesystem, shell, network or dialog permission — a permission added there should have to argue for
   itself in review.

3. **The CSP names `ipc:` and `http://ipc.localhost` in `connect-src`.** macOS routes Tauri's IPC through
   a custom scheme that a `default-src 'self'` policy happens to allow; Linux and Windows do not. Without
   those two sources every `invoke` fails on exactly the platforms Cairn targets most (Linux servers, the
   Pi tier) while working perfectly on the developer's Mac.

## Stopping a drug takes a reason

An order may be cancelled only by somebody **taking ownership** and **giving a rationale** (ADR-0060). So
`cease` requires non-empty reason text and authors as the unlocked clinician (ADR-0053). The CLI verb
still accepts neither ([#342](https://github.com/cairn-ehr/cairn-ehr/issues/342)) — that fix is
local-authoring-only by design, and this window is already on the right side of it.

## What this window deliberately does not do

No dedupe or merge UI, no John-Doe path, no identifier entry at registration
([#672](https://github.com/cairn-ehr/cairn-ehr/issues/672)), no dose editing, no prescribing, no
reconciliation. The pane/routing/freshness state machine in
`cairn-gui-shell` survived the iced retirement and is tested but not yet wired.

The §1.2 paper-parity **stopwatch figures are not yet measured**; that is a human act — see
[`results/RUNBOOK.md`](results/RUNBOOK.md) (section 8 for the front door). How often the step-3 prompt
truncates IS measured: [`results/2026-09-23-funnel-prompt-truncation.md`](results/2026-09-23-funnel-prompt-truncation.md).
