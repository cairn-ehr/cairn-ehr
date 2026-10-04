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
- It **never blocks a clinical write.** The queue hook cannot raise; the worker is a separate process
  that reads and writes only the matcher's own tables.
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
| `--bulk-threshold` | 500 | more than this many charts waiting: do one full sweep instead of per-chart checks |
| `--max-block-size` | 1000 | the largest block of look-alike charts the per-chart check will pair up (bigger blocks are reported as skipped, never silently truncated) |
| `--pace-ms` | 0 | pause between charts, to be gentle on a busy node |

While running, the worker listens for the database's `NOTIFY`, drains **newest change first** (a fresh
registration is checked within seconds even while an old backlog waits), then sleeps. If the connection
drops it reconnects with a backoff that doubles up to 60 s. A chart whose check raises is held for five
minutes and then retried; its notices stay, so it keeps reading as "not yet checked".

At start the worker compares its matcher configuration with the one recorded in the database. If they
differ (or it has never run), every chart is queued for a re-check, because a changed scorer may judge
old pairs differently.

## Is it keeping up? `cairn-node duplicate-check`

```bash
cairn-node duplicate-check                      # node-wide status
cairn-node duplicate-check --patient <uuid>     # also this one chart
```

It only reads. The node-wide line is one of four states:

| State | Line |
|---|---|
| Never run | `Duplicate check has never run on this node.` (with `— N charts waiting.` when notices are queued) |
| Behind | `Duplicate check is behind — last ran HH:MM; N charts waiting.` (or `— it has not finished a round yet;` when there is no last-ran time) |
| Running | `Duplicate check running — N charts waiting.` (with ` (re-checking all charts after a matcher update)` during a full re-check) |
| Up to date | `Duplicate check up to date — last ran HH:MM.` |

"Behind" means the **newest** waiting notice is more than five minutes old. A worker drains newest first,
so only a stopped or stuck worker lets the newest notice age; a large old backlog after a restore reads
as "running", not as an alarm. With `--patient` a second line says either
`This chart: duplicate check not yet run since its identity details last changed.` or
`This chart: duplicate check up to date.`

## What a restore or rebuild does

`reproject --rebuild`, a restore, or a new node's first pull rewrites the projections, so **every chart
queues a notice**. That is correct: the inputs changed. The worker sees a backlog above
`--bulk-threshold`, runs **one full sweep** (the sweep's blocking is far cheaper per chart than checking
each alone), clears the notices the sweep covered, and then works through anything left newest-first.
The status line reads "running" until the queue is empty.

## Keeping it alive

Both examples run the worker from a checkout at `/opt/cairn-ehr` and read the database settings from the
environment. Adjust the paths, host and role; keep the password out of the unit (use a `~/.pgpass` file
or the service manager's secret mechanism).

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
