# Person-menu reads at scale — 2026-11-08, macOS M3 Max

> Produced by `cargo run --release --example measure_person_menu` (module doc has the usage). One machine and no
> window or human, so this is the database and Rust share of the wait, not a RUNBOOK §26 stopwatch figure. It answers
> [#1021](https://github.com/cairn-ehr/cairn-ehr/issues/1021): does the person menu (ADR-0095) stay within RUNBOOK §26's
> **≤ 1 s from a pick to "Waiting for you: N"** as the history grows?

**Headline.** Before the fix the budget was missed at **5 000 notes**, about two months of one ward: a pick made while
the picker's counts load took 1.3 s, and a ward-year (33 000 notes) took 5.4 s. About 88% of that (the functions' own time) was the
same handful of facts about a signing key (whose it is, their name, role, confirmation, whether it is a human's),
looked up again for every row: about 1.4 million calls for 40 distinct keys. With each key looked up once
(`notes_sql!`'s `facts` CTE), output identical row for row, the reads are **5–6× faster**:

- a plain pick now fits the budget **up to 100 000 notes** (878 ms);
- a pick made while the picker's counts are still loading still misses it **from about a ward-year** (1 051 ms at
  33 000 notes) — that is the finding; the budget is not adjusted;
- the reads still grow linearly with all-time history. #1021's projection of open obligations is the fix for a
  facility-sized node.

## Rig

| | |
|---|---|
| Host / CPU / RAM | Apple M3 Max, 128 GB |
| OS | macOS (Darwin 27.2.0), aarch64 |
| Build profile | `--release` |
| PostgreSQL | 18.1 (Postgres.app) on `127.0.0.1:5532`, `cairn_pgx` as installed there; a fresh database per run |
| Node | provisioned by the real `cairn-node init` (sealed key) |

## The ward

40 clinicians. `dr-b` supervises 20 of them, and 5 of those 20 need a co-sign. There are 300 charts. Every note is
one part, signed by its writer through the production `sign_note`, with the writer and the chart taken in turn.
Each stage is cumulative and is timed with 5 repeats (the figure is the median):

- **bar** = the window's `person_menu::fresh`: `read_menus(Some("dr-b"))` + `menu_for` + `candidates_by_id`, in one
  REPEATABLE READ snapshot;
- **counts** = `whos_here_counts_impl`;
- **directory** = the picker's list;
- **pick behind counts** = the sum of all three, since the window's one connection serialises them.

**Distortion, stated.** Every note is seeded at the same moment, so `dr-b`'s group 3 (the last 7 days by HLC
wall) holds every supervisee note, where a real ward would show one week's. The SQL cost is the same either way
(it returns all-time rows); the Rust classification and the row count are over-stated. The rig files no vouches,
so the vouch subquery finds nothing. Its key lookups run once per vouch, not once per row.

## Before — key facts looked up per row (commit `5429c38a`)

| `clinical_note` rows | parts in counts read | dr-b's count | bar | counts | directory | pick behind counts |
|---:|---:|---:|---:|---:|---:|---:|
| 1 000 | 1 000 | 125 | 84 ms | 150 ms | 0 ms | 234 ms |
| 5 000 | 5 000 | 625 | 450 ms | 852 ms | 1 ms | **1 303 ms** |
| 10 000 | 10 000 | 1 250 | 593 ms | 1 126 ms | 1 ms | 1 721 ms |
| 20 000 | 20 000 | 2 500 | **1 112 ms** | 2 140 ms | 1 ms | 3 253 ms |
| 33 000 | 33 000 | 4 125 | 1 870 ms | 3 578 ms | 0 ms | 5 449 ms |
| 66 000 | 66 000 | 8 250 | 3 617 ms | 7 168 ms | 1 ms | 10 786 ms |
| 100 000 | 100 000 | 12 500 | 5 622 ms | 10 649 ms | 0 ms | 16 272 ms |

Linear at about 110 µs per part read. The profile behind the counts read at 100 000 notes, from `EXPLAIN ANALYZE`
(10.4 s) and `track_functions = 'all'`:

| function | calls | total |
|---|---:|---:|
| `cairn_note_key_actor` | 400 005 | 3.4 s |
| `cairn_note_signer_name` | 200 000 | 2.1 s |
| `cairn_key_confirmation` | 100 000 | 1.6 s |
| `cairn_note_key_role` | 100 000 | 1.5 s |
| `cairn_note_vouched_attester` | 500 000 | 1.1 s |
| `cairn_note_key_is_human` | 100 000 | 0.4 s |

The scan and join themselves took 0.2 s. The doors are `SECURITY DEFINER` with a pinned `search_path`, so the
planner cannot inline them, and each call is a full function invocation.

## After — key facts looked up once per distinct key

| `clinical_note` rows | parts in counts read | dr-b's count | bar | counts | directory | pick behind counts |
|---:|---:|---:|---:|---:|---:|---:|
| 1 000 | 1 000 | 125 | 13 ms | 16 ms | 0 ms | 30 ms |
| 5 000 | 5 000 | 625 | 39 ms | 76 ms | 0 ms | 116 ms |
| 10 000 | 10 000 | 1 250 | 71 ms | 252 ms | 0 ms | 324 ms |
| 20 000 | 20 000 | 2 500 | 178 ms | 390 ms | 0 ms | 569 ms |
| 33 000 | 33 000 | 4 125 | 353 ms | 697 ms | 1 ms | **1 051 ms** |
| 66 000 | 66 000 | 8 250 | 666 ms | 1 386 ms | 1 ms | 2 052 ms |
| 100 000 | 100 000 | 12 500 | 878 ms | 2 150 ms | 1 ms | 3 028 ms |

A second fresh node was seeded the same way. Timing the first node again with `--measure-only` gave the same
100 000-note row to within 7%: bar 933 ms, counts 2 182 ms. Before the rewrite was adopted, a scratch prototype was
checked against the old statement on the 100 000-note node, and `EXCEPT ALL` found no row differing in either
direction. The 27 DB-gated note suites (180 tests) pass on the rewrite.

## What is left, in #1021

- The **projection of open obligations** (one row per part that waits for someone, kept by the appliers), so the
  menu reads O(obligations), not O(history). Needed before a node of a ward-year or more uses the picker's counts.
- Cheaper levers, not taken here:
  - drop `closeChart`'s menu refresh when `person_menu_open` follows ("Open note" from an open chart reads the menu
    three times);
  - stop the picker's counts read from blocking a pick, e.g. by reading the counts after the pick or on their own
    connection.
- The **Pi** figures (the issue asks for both machines). The rig runs unchanged against a Pi node.
