# `iced-ui-spike` — Spike 0004 harness

Runnable harness for **[Spike 0004](../../docs/spikes/0004-iced-reference-ui-viability.md)** — is
**iced** viable as Cairn's **L3 reference-UI** framework
([eval 0004](../../docs/ecosystem/0004-reference-ui-framework-iced-vs-tauri.md))? It measures the three
bars an EHR cannot waive: **accessibility (A)**, **complex international text + IME (I)**, and
**paper-parity latency (L)** on the Pi-class software-render path.

This is a **fit-for-purpose L3 harness** ([§9.1](../../docs/spec/language-substrate.md)). It is a
standalone crate, deliberately **outside the root workspace**, so iced/wgpu/cosmic-text never enter the
`cairn-node` dependency tree. It never signs and never authors a real event — the UI is a pure L3
producer (ADR-0021).

## Layout — what runs where

| Part | Built by | Where it runs | Validates |
|---|---|---|---|
| `src/latency.rs` | default (`cargo test`) | **CI**, headless | **L1** — percentile summariser, exact against known vectors |
| `src/corpus.rs` | default | **CI**, headless | **I** structure — the multi-script name corpus + expectations |
| `src/form.rs` | default | **CI**, headless | **A1** structure — every focusable control has a role + label; emits the expected a11y tree |
| `src/shaping.rs` | `--features shaping` | CI **if fonts present**, else loud SKIP | **I1** — real shaping (rustybuzz, the shaper cosmic-text/iced wraps): non-tofu clusters |
| `src/bin/gui.rs` | `--features gui` | **operator** (workstation / Pi) | **A1/A2, I2/I3, L2** — the live screen-reader, IME, and latency passes |

The split is the point: the **pure claims are unit-tested in CI**; the claims that need a screen reader,
an IME, or a Pi are **operator-run** with the GUI, and their results are filed under `results/`.

## Run it

### CI / headless (no display needed)

```sh
cargo test                      # L1 + I-structure + A1-structure  (no toolkit compiled)
cargo test --features shaping   # adds I1: shapes the corpus with rustybuzz; SKIPs loudly if no font
cargo run --features gui -- --dump-a11y   # prints the expected a11y tree (claim A1 checklist) as JSON
```

`--dump-a11y` exits before opening a window, so it works in CI too.

### Operator passes (workstation with a display)

```sh
cargo run --features gui                    # the dense clinical form (default renderer)
cargo run --features gui -- --latency       # forces the tiny-skia software renderer; logs update→paint timings
```

The four name fields are **pre-filled from the corpus** (Latin, Arabic, Devanagari, Han) so RTL/shaping
is visible immediately and the Han field is ready for an IME-entry test.

## Operator scripts (the passes CI can't run)

Record outcomes in `results/` using `results/TEMPLATE.md`.

- **A1 — a11y tree.** Run `--dump-a11y`; confirm every `"focusable": true` entry is a control you expect.
  Then open an AT inspector ([Accerciser](https://gitlab.gnome.org/GNOME/accerciser) on Linux AT-SPI2)
  and confirm the live tree matches: each control has a role + the same label.
- **A2 — keyboard-only + screen reader.** Start Orca (Linux) or NVDA (Windows). With **no pointer**: Tab
  through identifier → 4 names → med rows (Remove) → Add medication → Save. Every stop must be announced
  with its label; the whole form must be completable blind. PASS = completed without sight.
- **I2 — RTL/bidi.** Confirm the Arabic field renders right-to-left, caret moves correctly, and editing
  mid-word keeps joining correct. Screenshot into `results/`.
- **I3 — CJK IME.** Clear the Han field; using a system IME (e.g. ibus-pinyin / fcitx), compose and commit
  Han text. PASS = the committed characters appear correctly.
- **L2 — Pi latency.** On Pi-class hardware: `cargo run --features gui -- --latency`, type steadily into a
  field for a few hundred keystrokes, read the rolling `[latency] … p95=…` line. PASS threshold: **p95 ≤
  100 ms** keystroke-to-paint (record the actual number regardless).

## What CI already shows (this box)

- `cargo test` → **16 passed** (latency + corpus + form).
- `cargo test --features shaping` → **18 passed**; the corpus shaping check **shaped all 4 scripts with
  zero tofu** here, because this box happens to carry GNU FreeSerif (Arabic + Devanagari) and WenQuanYi
  Zen Hei (Han). That is the **mechanical half of I1 passing** — it does **not** settle I2 (visual RTL) or
  I3 (IME), which still need the operator. On a font-less box the same check SKIPs loudly, never silently
  passes.
- `cargo run --features gui -- --dump-a11y` → emits the 13-entry expected a11y tree (9 focusable).

The remaining verdicts — A1/A2 (screen reader), I2/I3 (RTL + IME by eye), L2 (Pi p95) — are operator runs.

> **Pre-1.0 caveat.** The GUI targets **iced 0.14**. iced breaks API between minor releases (the
> `application(boot, …)` vs `application(title, …)` change between 0.13 and 0.14 already bit this harness);
> a future iced may need small touch-ups in `src/bin/gui.rs`. That churn is the fit-for-purpose risk the
> spike is measuring, and it is contained entirely in this L3 harness.
