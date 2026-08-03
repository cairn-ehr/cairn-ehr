# Paper-parity measurement — 2026-08-03, macOS dev rig — **PARTIAL**

> **This file records the WRITE half only. The §1.2 time budget is NOT yet measured.**
>
> The budget is about a *human gesture*: chart open → list read → unsigned lines signed. That
> needs a person at the keyboard with a stopwatch, and a live screen reader for the
> accessibility half. Neither was available to the session that built this slice, so both are
> owed — follow [`RUNBOOK.md`](RUNBOOK.md) and record the result in a new dated file from
> [`TEMPLATE.md`](TEMPLATE.md). This file exists so the *architecture-forced* half is a
> measured number rather than another seeded one, and so the runbook itself is known to work.

## What was actually done

Every command in `RUNBOOK.md` §0–§2 and §5 was executed end-to-end against a scratch node.
**Three of them were wrong as first written** and are now corrected in the runbook — which is
the main reason for running it:

- `init` requires `--name` and `--address`; the draft omitted both.
- `--key` is a **global** flag (before the subcommand). The draft passed it to `enroll-human`
  as a subcommand flag, which clap rejects.
- The connection/key environment variables were named inconsistently between steps.

A runbook nobody has executed is a runbook that does not work.

## Rig

| | |
|---|---|
| Host | Apple Silicon MacBook (aarch64-apple-darwin), Darwin 25.6.0 |
| Build profile | `--release` |
| PostgreSQL | 18 on `127.0.0.1:5532`, `cairn_pgx` 0.3.0 |
| Database | `cairn_measure`, freshly created and provisioned for this run |
| Node | `measure-rig`, genesis `1220c839…1639` |

## Chart under test

Seeded exactly as `RUNBOOK.md` §2 describes, five times over, one fresh patient per run:

| | |
|---|---|
| Lines displayed | 5 |
| Unsigned (sign-off targets) | 3 |
| Already signed by another clinician (Dr B, `2d234868`) | 2 |
| Reconciled groups | 0 |
| Groups missing from chart | 0 |

The chart read back exactly as the runbook predicts, including Dr B's short key id on the two
already-signed lines — so the "another clinician's signature is not reassigned to you" rule is
visible in the fixture, not just in the tests.

## Measured — `medication-sign-off`, whole CLI invocation

Five runs, three sign-off targets each, one fresh 5-drug chart per run:

| Run | 1 | 2 | 3 | 4 | 5 |
|---|---|---|---|---|---|
| ms | 224 | 217 | 228 | 222 | 234 |

Median **222 ms**, max **234 ms**.

**Read this as an upper bound, and a loose one.** It times the whole `cairn-node` process:
spawn, connect, *re-run all 44 migrations* (the loader replays `db/*.sql` on every connect),
read the chart, mint 3 HLCs, then seal+sign+submit 3 attestations in 3 transactions. The
window pays the connect-and-load cost **once at launch**, not per gesture, so the in-window
write is strictly smaller than this. No attempt was made to separate the two, because the
number that matters is the human gesture and that is still owed.

## Measured — `ui_gesture_timing`

Empty, as expected: the table is written by the **window**, not by the CLI, and no window
gesture has been performed against a real database yet. That asymmetry is deliberate — the
aggregate table exists to keep measuring the gesture in daily use, not to instrument a
one-off benchmark run.

## Step count against the paper counterpart (§1.2)

| Act | Paper *N* | Architecture-forced *M* | Observed UI *K* |
|---|---|---|---|
| Review a 5-drug list, sign 3 unsigned/stale lines | 3 signatures | **1** | not yet observed |
| Cease one drug | 2 (strike + initial/date) | **1** | not yet observed |

`M ≤ N` on both limbs, so there is no architecture defect to file under the #217 rule. The
five runs above confirm *M = 1* for sign-off directly: one invocation attested all three
threads, and the two lines Dr B had signed kept his signature.

**A correction to the plan's `K` for cease.** The plan projected `K = 1` (one click). The
built window charges **2**: type a reason, then press *Stop*. That is deliberate and it is
recorded here rather than quietly absorbed — ADR-0060's framing is that an order may be
cancelled only by somebody taking ownership **and giving a rationale**, so a one-click cease
would be a cancellation with no reason attached. `K = 2` still sits at the paper counterpart's
`N = 2` (strike + initial), so parity holds; it is the projection that was wrong, not the
implementation. Issue [#342](https://github.com/cairn-ehr/cairn-ehr/issues/342) tracks the
same gap at the CLI verb.

## Still owed (the reason this file says PARTIAL)

1. **The human gesture, timed** — both limbs, ≥5 samples each, against a real node.
2. **The accessibility pass** — a live screen-reader run through `RUNBOOK.md` §6's eight
   checks, verdict recorded per line.
3. **The verdict on the provisional budget** — whether the observed p95 falls inside 15 s /
   5 s. Until (1) exists, the budget remains a seeded figure and must be described that way.
