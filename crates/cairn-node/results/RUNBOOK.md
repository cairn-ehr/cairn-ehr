# DR restore paper-parity runbook (§1.2 / [#512](https://github.com/cairn-ehr/cairn-ehr/issues/512))

**What this measures.** DR slice 1's plan states a §1.2 budget and calls it unmeasured:

> a restore completes within **10 minutes** unattended after the operator's last keystroke; the
> operator needs **one secret** and **no knowledge of the dead node's configuration**

This runbook produces the time half as a **scaling curve**, not a single dot, with #512's own
100 000-event point as the headline. Everything timed is the **shipped CLI**, because the budget is a
promise about a ceremony and not about a function.

**What it does not measure, and why the omission is stated rather than buried.**

- **Seeding is not measured.** It exists only to produce a medium at a realistic scale. A rig that
  folded it in would be timing the write path, which has its own budget elsewhere.
- **Finding and attaching the medium is not measured.** It is a physical act with a real
  counterpart — walking to the off-site box — and no software here can time it. It is counted in the
  step count below and excluded from the clock, and every recorded run must repeat that exclusion.
- **The cognitive-load half is not timed at all**, because it is structural. *"No knowledge of the
  dead node's config"* is pinned mechanically by
  [`restore_needs_nothing_about_the_dead_node.rs`](../tests/restore_needs_nothing_about_the_dead_node.rs),
  which asserts against the real `--help` that `restore` requires only the medium and the new
  database, and that `--superseded-node` stays optional.

> [!IMPORTANT]
> **If a measurement falls outside the budget, that IS the finding.** File it against #512. Never
> adjust the budget to fit a run — house rule 7, and the rule this file exists to serve.

---

## 0. Prerequisites

A PostgreSQL cluster (≥ 18) with `cairn_pgx` installed — `scripts/pg-target.sh` discovers one rather
than assuming a port. Release builds of `cairn-node` and of the seeding example:

```bash
cargo build --release -p cairn-node
cargo build --release -p cairn-node --example seed_measurement_corpus
```

> Build `--release`. A debug restore measures the compiler, not the ceremony.

## 1. Run the rig

```bash
python3 scripts/measure_dr_restore.py --sizes 100,1000,2500,5000
```

Sizes are **patient counts**. Each patient contributes 3 demographic events (registration, name, date
of birth — ADR-0061's search-carrying act) plus `--meds-per-patient` **born-sealed** clinical events,
so the default `17` puts the 5 000-patient point at just over 100 000 events.

The rig runs, per size:

1. `cairn-node init` — the **real** provisioning ceremony, sealed. It mints the key, the local-state
   escrow and the unwrap key, and prints the recovery code the restore will later ask for. A
   `--insecure-plaintext` node has **no** escrow, so `backup` writes no sealed `CAIRNL1` export and the
   restore would measure the degraded, custody-less path.
2. `cairn-node patient-register` once, to enroll the node's own `device` actor. That is an owner
   ceremony living in the CLI, and re-spelling it inside the seeder would be a second copy of a
   ceremony — the mirror-list defect class this repo keeps paying for.
3. The seeding example, in-process on one connection, through the production orchestrators.
4. `cairn-node backup` — timed and reported, because it is the number
   [#552](https://github.com/cairn-ehr/cairn-ehr/issues/552) is about.
5. `cairn-node restore` into a freshly-created database — **the measured leg**.

The rig **refuses to record a timing for an incomplete restore**. A restore that applied nothing is
fast, and a clean-looking summary over an empty restore is #500's own signature, so a fast wrong
number is precisely what would end up quoted against the budget.

## 2. The pseudo-terminal, and the finding behind it

The rig drives `restore` on a pty. That is not tidiness. The old node's recovery code is read through
`rpassword`, which opens `/dev/tty` and **fails on any non-tty** — and unlike the new key's
passphrase it has **no flag and no environment variable**. Piping it does not merely fail to work: the
read errors, the export never opens, and the restore recovers zero patients while exiting non-zero.

So **a DR drill cannot be scripted or run from cron**, which is
[#572](https://github.com/cairn-ehr/cairn-ehr/issues/572). Anyone re-running this runbook by hand will
simply type the code at the prompt and see none of that.

## 3. Verify the restore actually opened a body

The rig checks the restore's own summary. That is necessary and not sufficient — rows arriving is not
the same claim as a body opening, which is the lesson slice 2d recorded in as many words. Confirm by
hand at least once per recorded run:

```bash
psql "$DST_CONN" -tAc "SELECT count(*) FROM event_dek"
psql "$DST_CONN" -tAc "SELECT count(*) FROM event_clear"
psql "$DST_CONN" -tAc "SELECT left(twin,60) FROM event_clear LIMIT 2"
```

The third command must print readable medication text. A double-wrapped DEK would leave the first two
counts agreeing and the plaintext unreadable.

## 4. Record the result

Copy [`TEMPLATE.md`](TEMPLATE.md) to `YYYY-MM-DD-<host>.md` and fill it in. Record what you measured,
not what the budget hoped for.

## Step count against the paper counterpart (§1.2)

**Paper counterpart:** the off-site duplicate chart — the practice that copies its records, keeps the
copy in another building, and carries the box back after a fire. *N* = **2** human acts: fetch the box,
shelve it.

DR slice 1's plan put the architecture-forced count at *M* = **3**, the third act being *"confirm the
echoed identity when provenance is not sole-enroll-signed."*
**[ADR-0068](../../../docs/spec/decisions/0068-provenance-warns-never-gates-on-the-restore-path.md)
deleted that act**: provenance warns and never gates, so there is no confirmation to give. Re-derived
against the shipped command, the acts are:

| # | Act | Counterpart on paper |
|---|---|---|
| 1 | Attach the medium | fetch the box |
| 2 | Run `cairn-node restore --from <medium>`, entering the operational passphrase for the new key | — |
| 3 | Enter the **old** node's recovery code at the prompt | — |

So *M* = **3** still, but for a **different reason than the plan recorded**, and the reason matters:
the third act is now the **recovery-code entry**, which DR slice 1's plan had assumed was bundled into
act 2 as *"one interactive ceremony"*. It is not bundled — it is a second, separately-prompted secret
arriving after the node plane has already been applied.

`M > N` therefore **stands, filed and not argued away**, and #512 stays open. What changed is which act
is the excess one, and that is a better-posed problem than the one the plan filed: an identity
confirmation has no paper counterpart and could never be bundled away, whereas a second secret prompt
plausibly could be — which is what makes *K* = **2** still believable.
