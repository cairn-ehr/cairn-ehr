# `cairn-gui-tauri` — the reference med-list window

Cairn's first runnable clinical surface ([#288](https://github.com/cairn-ehr/cairn-ehr/issues/288)): a
Tauri 2 window on **one patient's medication chart**, with whole-list sign-off as one human gesture and
per-row cease.

```bash
# Fixture mode — no database, no writes. What the accessibility pass and the demo use.
cargo run -p cairn-gui-tauri -- --mock --patient 00000000-0000-0000-0000-000000000001

# Against a real node.
cargo run --release -p cairn-gui-tauri -- \
    --patient <uuid> --conn "$CONN" --key node.key --attester-key dr-a.key
```

## Where the decisions live

| Question | Answer, and where |
|---|---|
| What does a sign-off gesture attest? | `cairn_medication_view::sign_off_targets` — **one** definition, shared with the node's orchestrator |
| What does the window display? | `cairn_gui_tab_medications::build_view` — every clinical display decision, in Rust, under `cargo test` |
| What does the webview decide? | **Nothing.** It renders `MedListView` and calls commands |
| Why is a defect on one line not fatal? | [ADR-0060](../../docs/spec/decisions/0060-partial-validity-a-defect-on-one-line-never-invalidates-another.md) |

## Three things that will surprise a newcomer

1. **No npm, no bundler, no `node_modules`.** `withGlobalTauri` puts `invoke` on `window`, so
   `src-ui/main.js` *is* the frontend — hand-written, unminified, and exactly what runs. The cost is no
   type checking, paid for by a test in `src/commands.rs` that scans `main.js` for every field access and
   asserts the backend actually sends it. It is verified to fail on a renamed field. Do not "improve" the
   frontend by adding a build step without reading
   [#332](https://github.com/cairn-ehr/cairn-ehr/issues/332) first.

2. **`gen/` is generated and gitignored**; `capabilities/default.json` grants `core:default` and nothing
   else. Every clinical action goes through a command in `src/commands.rs`, so the webview needs no
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

No patient picker (`--patient` at launch; the §5.3/§5.8 search-before-create funnel is unbuilt), no dose
editing, no prescribing, no reconciliation. The pane/routing/freshness state machine in
`cairn-gui-shell` survived the iced retirement and is tested but not yet wired.

The §1.2 paper-parity **time budget is not yet measured** — see
[`results/RUNBOOK.md`](results/RUNBOOK.md).
