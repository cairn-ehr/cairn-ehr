# Running the duplicate check

Cairn looks for duplicate charts after the fact, so that a busy front desk never has to. A small
standalone program, the **commit-time worker** (`cairn-matcher watch`), does the looking. This page is
for the person who operates a node: what the worker does, how to run it, how to see whether it is
keeping up, and how to keep it alive.

## What the worker does, and never does

Whenever a chart's identity evidence changes (a name, a demographic or an identifier), the database
appends a **notice** to a queue (`match_pending`, db/056). The worker drains that queue: for each chart
it finds the other charts that might be the same person, scores each pair with the advisory matcher,
and writes the result as `match_proposal` rows.

- It **never links** anyone. A proposal is a suggestion; only a human decision (R2's gesture) links
  charts. Identity is a claim, never a fact.
- It **never makes a clinical write wait on it.** The queue hook cannot raise on its own and never
  waits on the worker; it adds one small insert and a brief commit-time serialisation (PostgreSQL's
  `NOTIFY` takes a short cluster-wide lock while the writing transaction commits). The worker is a
  separate process that reads the identity projections and writes only the matcher's own tables
  (`match_proposal`, `match_pending`, `match_worker_state`).
- It **never claims "no duplicates" falsely.** A chart stays "not yet checked" for as long as one of its
  notices is still in the queue, and a worker that has never run is reported as such.

The worker is a standalone process you run on the node (or on a host that can reach its database). It
is deliberately not started by `cairn-sync`, so the safety-critical daemon is never tied to the
advisory tier.

## The database role

The worker connects as a role that holds `cairn_agent`. Grant it to the login role you use:

```sql
GRANT cairn_agent TO matcher_login;   -- the login role the worker connects as
```

`cairn_agent` is the advisory-actor role (ADR-0030): db/056 grants it the queue and worker-state
tables the worker writes, alongside the matcher's own proposal tables.

## Running it: `cairn-matcher watch`

From the repository root:

```bash
uv run --project matcher --extra pipeline cairn-matcher watch
```

With no `--dsn` the standard libpq environment is used (`PGHOST`, `PGPORT`, `PGUSER`, `PGDATABASE`,
`PGPASSWORD`, ...). Flags:

| Flag | Default | Meaning |
|---|---|---|
| `--dsn` | empty (use `PG*`) | libpq connection string |
| `--once` | off | drain the queue once and exit (0 = clean, 1 = a chart failed, 2 = database unreachable) |
| `--poll-seconds` | 60 | how long to wait for a notification before looking again (a backstop for a missed `NOTIFY`) |
| `--bulk-threshold` | 30 | more than this many charts waiting: do one full sweep instead of per-chart checks (30 comes from the measured break-even; see below) |
| `--max-block-size` | 1000 | the largest block of look-alike charts the per-chart check will pair up (bigger blocks are skipped and logged as a warning, never silently truncated) |
| `--pace-ms` | 0 | pause between charts, to be gentle on a busy node |

While running, the worker listens for the database's `NOTIFY`, drains **newest change first**, then
sleeps. It re-reads the queue after every chart, so a fresh registration is checked next, ahead of older
changes; at 10 000 charts one check takes ~9 s on the development machine (#725), and a sweep already
running finishes first. If the connection drops it reconnects with a backoff that doubles up to 60 s. A
chart whose check raises is held for five minutes and then retried; its notices stay, so it keeps reading
as "not yet checked".

**Very large look-alike blocks.** A block is a group of charts that share one blocking value (a name
token, a date of birth, an identifier). The two modes cap it differently:

- a **per-chart check** pairs the chart with every member of a block of up to `--max-block-size`
  (default 1000, `DEFAULT_TARGETED_CAP`) charts;
- a **bulk sweep** pairs every member with every other, so it keeps only blocks of up to 100 charts
  (`sweep_block_size`).

A bigger block is skipped, never silently: the per-chart check logs one warning per chart naming each
skipped block (its pass, value and size), and the sweep logs how many blocks it skipped. So after a
first run, a restore, a rebuild or a matcher update (all swept), a chart whose look-alike block has
101–1000 members is cleared with fewer comparisons than a per-chart check would have made. It still
reads "up to date"; whether it should stay "not yet checked" is an open design question.

At start the worker compares its matcher configuration with the one recorded in the database. If they
differ (or it has never run), every chart is queued for a re-check, because a changed scorer may judge
old pairs differently.

## Is it keeping up? `cairn-node duplicate-check`

```bash
cairn-node duplicate-check                      # node-wide status
cairn-node duplicate-check --patient <uuid>     # also this one chart
```

It only reads. It does not load or upgrade the schema (that is the node's own start-up), so it is
cheap to run from cron, and on a node whose schema predates db/056 it fails with an error rather than
printing a status. The node-wide line is one of four states:

| State | Line |
|---|---|
| Never run | `Duplicate check has never run on this node.` — or, when notices are queued, `Duplicate check has never run on this node — 1 chart waiting.` / `… — N charts waiting.` |
| Behind | `Duplicate check is behind — last ran HH:MM; N charts waiting.` (or `— it has not completed a check yet;` when there is no last-ran time) |
| Running | `Duplicate check running — N charts waiting.` (with ` (re-checking every chart: first run or matcher update)` during a full re-check) |
| Up to date | `Duplicate check up to date — last ran HH:MM.` (or `Duplicate check up to date.` when the worker has no last-ran time yet) |

"Behind" means **a change has been waiting for more than five minutes and the worker has finished no
work in those five minutes** — measured from the oldest waiting notice or the worker's last finished
work, whichever is later. Only finished work counts: the worker records it after every chart it checks
and about every 30 seconds while a sweep is successfully scoring pairs. Starting a sweep does not count,
and neither the sweep's first phase (finding the candidate pairs), a single slow pair, nor a pair that
fails records anything. So "behind" means the worker is stopped, keeps failing and restarting, fails on
every pair it tries (a missing grant, say — check its log), or is stuck in one piece of work that
long. New changes arriving do not reset it: on a busy node a stopped worker still reads "behind"
five minutes after the first change it failed to check. A healthy worker working through a restore's
backlog reads "running", however old the queued notices are, because it keeps finishing work; a shrinking `N charts waiting` between two runs of the command (and, after a sweep, the worker's
`swept a backlog of …` log line) shows it working. "Last ran HH:MM" is the worker's last finished work,
not its last sign of life: a worker that keeps failing and restarting is active, but its time stays put.
The time is the database's local time, and it carries the date (`last ran 2026-10-03 09:41`) whenever
that work was not today, so a worker that stopped days ago never reads as this morning.
With `--patient` a second line says either
`This chart: duplicate check not yet run since its identity details last changed.` or
`This chart: duplicate check up to date.`

## What a restore or rebuild does

`reproject --rebuild`, a restore, or a new node's first pull rewrites the projections, so **every chart
queues a notice**. That is correct: the inputs changed. The worker sees a backlog above
`--bulk-threshold`, runs **one full sweep**, clears the notices the sweep covered, and then works through
anything left newest-first. One sweep is the cheaper way through a large backlog: measured on an Apple M3
Max with 10 000 records, one sweep took about 2 minutes, while one per-chart check took about 9 seconds,
so the sweep cost the same as about 15 charts checked singly (about 83 at 2 000 records). The default
threshold of 30 sits between those break-evens. While the worker makes progress the status line reads
"running"; it reads "up to date" once the queue is empty.

## Keeping it alive

Both examples run the worker from a checkout at `/opt/cairn-ehr` and read the database settings from the
environment. Adjust the paths, host and role; keep the password out of the unit (use a `~/.pgpass` file
or the service manager's secret mechanism).

Two things to adjust before you load either example:

- **The `uv` path.** Service managers start programs with a minimal `PATH`, so give `uv`'s full path:
  `/opt/homebrew/bin/uv` on Apple Silicon with Homebrew, `/usr/local/bin/uv` on Intel Homebrew, often
  `~/.local/bin/uv` for a per-user install (`command -v uv` tells you).
- **Logs and first start.** The `/tmp` log path is an example only (`/tmp` is cleared on reboot). Under
  systemd the service user needs a home directory, because `uv` keeps its cache there, and the first start
  needs network access to fetch the dependencies unless that user's cache has been warmed (run the same
  `uv run …` once by hand as that user).

### macOS: launchd

Save as `~/Library/LaunchAgents/org.cairn.matcher-watch.plist`, then
`launchctl load ~/Library/LaunchAgents/org.cairn.matcher-watch.plist`.

```xml
<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
<plist version="1.0">
<dict>
  <key>Label</key><string>org.cairn.matcher-watch</string>
  <key>ProgramArguments</key>
  <array>
    <string>/usr/local/bin/uv</string>
    <string>run</string>
    <string>--project</string><string>/opt/cairn-ehr/matcher</string>
    <string>--extra</string><string>pipeline</string>
    <string>cairn-matcher</string><string>watch</string>
  </array>
  <key>EnvironmentVariables</key>
  <dict>
    <key>PGHOST</key><string>127.0.0.1</string>
    <key>PGPORT</key><string>5432</string>
    <key>PGUSER</key><string>matcher_login</string>
    <key>PGDATABASE</key><string>cairn</string>
  </dict>
  <key>RunAtLoad</key><true/>
  <key>KeepAlive</key><true/>
  <key>StandardErrorPath</key><string>/tmp/cairn-matcher-watch.log</string>
</dict>
</plist>
```

### Linux: systemd

Save as `/etc/systemd/system/cairn-matcher-watch.service`, then
`systemctl enable --now cairn-matcher-watch`.

```ini
[Unit]
Description=Cairn commit-time duplicate check (advisory; never links)
After=network.target postgresql.service

[Service]
User=cairn
Environment=PGHOST=127.0.0.1
Environment=PGPORT=5432
Environment=PGUSER=matcher_login
Environment=PGDATABASE=cairn
ExecStart=/usr/local/bin/uv run --project /opt/cairn-ehr/matcher --extra pipeline cairn-matcher watch
Restart=always
RestartSec=5

[Install]
WantedBy=multi-user.target
```

The worker has no monitoring exit code or metric yet; supervise it with `cairn-node duplicate-check`
(for example from a cron job that alerts on the "behind" line).
