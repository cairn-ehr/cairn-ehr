# DR restore paper-parity measurement — YYYY-MM-DD, \<host>

> Copy to `YYYY-MM-DD-<host>.md`. Follow [`RUNBOOK.md`](RUNBOOK.md). **Record what you measured, not
> what the budget hoped for** — a figure outside the budget is the finding, and it gets filed against
> [#512](https://github.com/cairn-ehr/cairn-ehr/issues/512), never explained away.

## Rig

| | |
|---|---|
| Host / CPU / RAM | |
| OS | |
| Build profile | `--release` |
| PostgreSQL version + `cairn_pgx` | |
| Storage | (NVMe / SATA SSD / spinning — a restore is write-heavy, so say) |
| Operator | |

## Corpus shape

Each patient contributes 3 demographic events plus `--meds-per-patient` **born-sealed** clinical
events. State the mix: a corpus of demographics only carries no `event_dek` rows, so the restore's
per-record unwrap and re-wrap — the expensive half — would never run.

| | |
|---|---|
| Meds per patient | |
| Sealed clinical share of the medium | |
| Human author enrolled? | (should be yes — a restored node resolves every author through `actor_current`) |

## Measured — the curve

Only the **Restore** column is measured against the budget. Seed is context; Backup is reported for
[#552](https://github.com/cairn-ehr/cairn-ehr/issues/552).

| Events on medium | Seed (s) | Backup (s) | **Restore (s)** | Applied | ≤ 10 min |
|---:|---:|---:|---:|---:|:--|
| | | | | | |

**Headline point (#512's own):** a 100 000-event medium restores in ______ s against a budget of 600 s.

**Shape:** is restore time linear in the medium, or worse? Say which, and give the per-event cost at
the top and bottom of the curve — a curve that bends is a finding even when every point passes.

## Verified — a body actually opened

Rows arriving is not the same claim as a body opening. Record the three checks from runbook §3:

| | |
|---|---|
| `event_dek` rows | |
| `event_clear` rows | |
| A twin read back | (paste one line of real medication text) |

## Step count against the paper counterpart (§1.2)

Paper counterpart: the off-site duplicate chart. *N* = 2 (fetch the box; shelve it).

| # | Architecture-forced act | Bundled in a UI? |
|---|---|---|
| 1 | Attach the medium | no — physical |
| 2 | Run `restore`, entering the new key's operational passphrase | |
| 3 | Enter the old node's recovery code at the prompt | |

*M* = ____ against *N* = 2. If `M > N`, it is **filed, not argued away** (house rule 7).

## Findings

Anything outside budget, and anything the run surfaced that the budget does not cover. One line each,
with the issue number. **If there are none, say so explicitly** rather than leaving the section empty.
