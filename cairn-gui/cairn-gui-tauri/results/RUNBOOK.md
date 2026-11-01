# Paper-parity measurement runbook (§1.2 / [#288](https://github.com/cairn-ehr/cairn-ehr/issues/288))

**What this measures, and what it does not.** The plan's time budget — chart open → list
rendered → unsigned lines signed **≤ 15 s** for a 5-drug list, one cease **≤ 5 s** — was
*seeded, not measured*. This runbook produces the measured figure. It is one operator on one
machine: an honest data point, not a study.

**Explicitly excluded: finding the patient.** The window launches with `--patient <uuid>`
so sections 1–7 time the chart gestures alone. Every recorded run of those sections must repeat
that exclusion, because the paper counterpart — picking the right chart off a trolley — is a
real act with a real wrong-chart hazard, and a figure that quietly omits it would flatter the
architecture. **Finding the patient is measured separately, in section 8** (the §5.3/§5.8
funnel, built in slice 2c).

**The accessibility pass is a separate act.** It is a live screen-reader run (VoiceOver on
macOS, Orca on Linux, NVDA on Windows) against the same window, recorded in the same file.
Automating DOM assertions in CI is [#332](https://github.com/cairn-ehr/cairn-ehr/issues/332)
and needs a JS-toolchain decision this slice deliberately did not take.

---

## 0. Prerequisites

A node database with `cairn_pgx` installed, and this node's key. The rig is a throwaway Postgres cluster
of its OWN, on another port, made with a password and `scram-sha-256` from its first start: never switch the
shared development cluster (:5532, also `CAIRN_TEST_PG`, see getting-started) away from `trust`, because every
DB-gated test suite connects to it with no password. The binaries are the PG18 install `cairn_pgx` was installed
into (`pg_config --bindir`), so `CREATE EXTENSION cairn_pgx` works in the new cluster too. The `init` and later
`cairn-node` flags were run end-to-end on 2026-08-03 against a `trust` rig; the cluster steps (`initdb` to `start`)
and the window's own login (below) were written on 2026-11-01 for #898/#899 and have not yet been run end-to-end,
so a first run that needs a change is a finding:

```bash
export PGBIN="$(pg_config --bindir)"     # the PG18 install cairn_pgx is installed into
export MDATA=/tmp/cairn-measure-pg MPORT=5533
read -rs OWNER_PW && export OWNER_PW     # the owner's password; no spaces or quotes (it goes in a connection string)
( umask 077; printf '%s\n' "$OWNER_PW" > "$MDATA.pw" )
# The owner's password is set, and every pg_hba.conf line is scram-sha-256, before the server first starts.
"$PGBIN/initdb" -D "$MDATA" -U "$USER" --auth=scram-sha-256 --pwfile="$MDATA.pw"
rm "$MDATA.pw"
"$PGBIN/pg_ctl" -D "$MDATA" -o "-p $MPORT" -l "$MDATA.log" start

export PGPASSWORD="$OWNER_PW"            # psql reads it; tokio-postgres (cairn-node, the window) does not
export CONN="host=127.0.0.1 port=$MPORT user=$USER password=$OWNER_PW dbname=cairn_measure"
export NODE_KEY=/tmp/measure-node.key

"$PGBIN/psql" -h 127.0.0.1 -p "$MPORT" -d postgres -c "CREATE DATABASE cairn_measure"
"$PGBIN/psql" "$CONN" -c "CREATE EXTENSION IF NOT EXISTS cairn_pgx"

cairn-node --conn "$CONN" --key "$NODE_KEY" \
    init --name measure-rig --address 127.0.0.1:0 --insecure-plaintext   # test rig only
```

When the measurement is done: `"$PGBIN/pg_ctl" -D "$MDATA" stop && rm -rf "$MDATA" "$MDATA.log"`.

> `--key` is a **global** flag: it comes before the subcommand, not after it. Every command
> below follows that shape.

**The window runs on its own login, not the owner's** ([ADR-0084](../../../docs/spec/decisions/0084-the-window-never-holds-the-owners-keys.md),
#898/#899). Three consequences for this rig:

1. **The cluster must not trust anyone.** `provision-window-role` refuses while any `pg_hba.conf` rule (an included
   file's too) says `trust` (it names the file and line), and refuses rules changed since the server last loaded
   them. The cluster above is made that way from its first start, so neither refusal should fire; if one does, the
   cluster is not the one this section made.
2. **The owner provisions the window's login once**, after `init` (it also installs or upgrades the schema, which the
   window never does). The owner must be a database superuser (it reads `pg_hba_file_rules`). Choose the window's
   password and export it for the step (otherwise it is prompted for, twice):

   ```bash
   export CAIRN_WINDOW_DB_PASSWORD='…'
   cairn-node --conn "$CONN" provision-window-role --role cairn_window_login
   export WINDOW_CONN="host=127.0.0.1 port=$MPORT user=cairn_window_login password=$CAIRN_WINDOW_DB_PASSWORD dbname=cairn_measure"
   ```

   Re-run it after every schema-changing build: a window refuses to start on a database its build has not been
   prepared for, and names this command.
3. **Every window launch below uses `"$WINDOW_CONN"`**; every `cairn-node` and `psql` step keeps the owner `"$CONN"`.
   The window gets it as `CAIRN_CONN="$WINDOW_CONN" cargo run …`, not `--conn`: the string carries a password, and a
   command-line argument is visible to every account on the machine (`ps`), an environment variable only to yours (and root).
   `cairn-node` reads `CAIRN_CONN` too; on a machine other people log into, run its lines the same way.
   The connection must be on this machine (`127.0.0.1`, `localhost` or a socket): the window refuses any other host.
   A window started with `CAIRN_CONN="$CONN"` still opens and reads, but every signature and "Enrol staff" is refused with
   "more access than a window may have", and the reason is printed on the terminal's stderr.

## 1. Enrol the clinician who will sign

The window signs as an **enrolled human actor** (ADR-0053). Since the sign-in slice the window
opens no key file: each act carries the clinician's **staff ID and passphrase**, and the node's key
vault (db/061) releases their key for that act. A vault member is enrolled by a superuser
(`cairn-node vault-bootstrap-superuser` makes the first one at the console; the window's *Enrol
staff* form makes the rest — section 16 walks both). The `enroll-human` key files below are still what the
`cairn-node` CLI verbs (`link-charts`, …) sign with. Two clinicians need two *distinguishing determinants* — enrolling both as a bare
`{"role":"clinician"}` collides into one `actor_id` and is refused
([ADR-0044](../../../docs/spec/decisions/0044-enroll-fail-closed-on-actor-id-collision.md)).

```bash
cairn-node --conn "$CONN" --key /tmp/dr-a.key enroll-human --handle dr-a --insecure-plaintext
cairn-node --conn "$CONN" --key /tmp/dr-b.key enroll-human --handle dr-b --insecure-plaintext
```

> The launches below pass `--key-hold-seconds 900` (a personal workstation: one passphrase,
> then the key is held for the next acts). A shared terminal is the default, 0, where every act
> carries the staff ID and passphrase; say which you measured. Typing the credential is part of
> the gesture whenever no key was held.

## 2. Seed a five-drug chart, three of which are unsigned

The benchmark's row: *review a 5-drug list, sign 3 unsigned/stale lines.* So two lines must
already carry **someone else's** current signature — that is what proves the gesture leaves
another clinician's vouch alone rather than reassigning it.

Since [#345](https://github.com/cairn-ehr/cairn-ehr/issues/345) a chart must be **registered**
before anything can be recorded about it — the §5.3/§5.8 precedence rule, enforced in the
database. A hand-minted `uuidgen` id is refused by `medication-assert`, which is the point: you
cannot write on a chart nobody made, exactly as on paper. `patient-register` runs the
search-before-create funnel and prints the new chart id on the line that starts
`registered patient ` (the registration event's id follows on the next line, so read the chart
by that prefix, never by `tail -1`).

```bash
cnode() { cairn-node --conn "$CONN" --key "$NODE_KEY" "$@"; }   # a function: an unquoted $CONN would word-split
PATIENT=$(cnode patient-register --name "Bench Patient" --birth-date 1980-01-01 \
    --confirm-new | sed -n 's/^registered patient //p')
[ -n "$PATIENT" ] || { echo "registration failed — read the output above" >&2; exit 1; }

# Three lines nobody has signed.
cnode medication-assert "$PATIENT" \
    --term atorvastatin --dose-amount 40 --dose-unit mg --formulation tablet
cnode medication-assert "$PATIENT" \
    --term metformin --dose-amount 1 --dose-unit g --formulation tablet
cnode medication-assert "$PATIENT" \
    --term sertraline --dose-amount 50 --dose-unit mg --formulation tablet

# Two lines Dr B has already signed — author-time attestation in one act.
cnode medication-assert "$PATIENT" \
    --term amlodipine --dose-amount 5 --dose-unit mg --formulation tablet \
    --attest-as /tmp/dr-b.key
cnode medication-assert "$PATIENT" \
    --term perindopril --dose-amount 4 --dose-unit mg --formulation tablet \
    --attest-as /tmp/dr-b.key

echo "PATIENT=$PATIENT"
```

Confirm the chart before measuring anything — if it does not read the way you expect, the
measurement measures the wrong thing:

```bash
cnode medication-list "$PATIENT"
```

Expect exactly this shape — five lines, two carrying Dr B's short key id:

```
amlodipine 5 mg     [current] — signed by 2d234868
atorvastatin 40 mg  [current] — unsigned
metformin 1 g       [current] — unsigned
perindopril 4 mg    [current] — signed by 2d234868
sertraline 50 mg    [current] — unsigned
```

**If it reports groups missing from the chart, stop** and clear that first: you would be
timing a gesture over a chart the node already says is incomplete.

## 3. Launch the window

```bash
cd cairn-gui
CAIRN_CONN="$WINDOW_CONN" cargo run --release -p cairn-gui-tauri -- \
    --patient "$PATIENT" \
    --key "$NODE_KEY" --key-hold-seconds 900
```

`--release` matters: a debug build measures the compiler's tempo, not the design's.

## 4. The measured gestures

Start the stopwatch **when the window appears**, not when the command is typed — process
start-up is not a clinical act and is not what the budget is about.

1. **Review and sign.** Read the five lines, enter dr-a's staff ID and passphrase (unless a key
   is already held), press *Sign off 3 unsigned medication(s)*. Stop the clock when the outcome line reports the result. Record the wall
   time, and note whether the count on the button matched the number of lines you judged to
   need a signature.
2. **Cease one drug.** Type a reason into a current row and press *Stop*. Stop the clock when
   the outcome line reports it.

Repeat each gesture at least five times on fresh charts (re-run §2), because the aggregates
below are running estimates and a single sample tells you nothing about the tail.

## 5. Read the aggregates back

The node records what each *write* cost, with no user, no patient and no timestamp — see the
header of [`db/044_ui_gesture_timing.sql`](../../../db/044_ui_gesture_timing.sql) for why the
absent columns are the design.

```bash
psql "$CONN" -c "SELECT * FROM ui_gesture_timing ORDER BY gesture_kind, size_bucket"
```

These are the **write** costs. Your stopwatch figure is the **whole gesture** including human
review, and it is the one the §1.2 budget is about; the table exists so the write half keeps
being measured in use, long after this runbook is forgotten.

## 6. The accessibility pass

Run the window again with `--mock` (no database, and the fixture chart deliberately carries a
cross-patient line and an invisible group so the warnings are exercised):

```bash
cargo run -p cairn-gui-tauri -- --mock --patient 00000000-0000-0000-0000-000000000001
```

With the screen reader on, and **keyboard only**, confirm each of these. Record a verdict per
line, not one overall pass:

- [ ] Every drug line announces drug, dose, status **and whose signature it carries** in one
      utterance — not by hunting cell to cell.
- [ ] The chart-level warnings are announced **before** the table content.
- [ ] Each *Stop* button announces the drug it stops, not a bare "Stop".
- [ ] Each reason field announces which drug it belongs to.
- [ ] The sign-off button announces the real number of threads it will sign.
- [ ] Every control is reachable by Tab, and the focus ring is visible at every stop.
- [ ] A line that will be signed is identifiable **without colour** (its signature cell says
      "will be signed").
- [ ] A ceased line is identifiable without colour (its status cell says "ceased").

**A linked chart (ADR-0076 R1, added 2026-09-27).** The `--mock` fixture chart is never linked, so
these need the live window opened on a chart that shares a link component with another. The
identity header then lists each member chart's own name, date of birth and trust state (no winner
is chosen), and every drug line names the chart it was recorded on. Confirm, again per line:

- [ ] The header's member lines are announced with the identity header, before the table.
- [ ] Each drug line's source chart is announced in the same utterance as the drug. Record how
      long it takes to hear: today it is a full chart uuid on every line, a known cost filed as
      [#691](https://github.com/cairn-ehr/cairn-ehr/issues/691).
- [ ] The same drug recorded on both linked charts shows as two lines, **both flagged** as a
      possible duplicate — never two silent lines.
- [ ] If a member's identity cannot be read, the list still shows, with a warning announced.

## 7. Record it

Copy [`TEMPLATE.md`](TEMPLATE.md) to `YYYY-MM-DD-<host>.md` and fill it in. **Record the
number you measured, whatever it is.** If the observed p95 falls outside the provisional
15 s / 5 s budget, that is the finding — file an issue and write it down. Adjusting the
budget to match the result would make the benchmark unfalsifiable, which is the one thing
§1.2 cannot afford.

## 8. The front door: find or register a patient (§5.3/§5.8, slice 2c)

The funnel's own §1.2 figure. **The stopwatch half is a human act**: the machine half is already
measured (search latency, #639's figures in ROADMAP; how often the step-3 prompt truncates and
whether the duplicate survives,
[`2026-09-23-funnel-prompt-truncation.md`](2026-09-23-funnel-prompt-truncation.md)).

**Seed.** On the node from section 0, register a handful of charts the operator can look for
(no `enroll-human` needed: registration is signed by the node's own key). Each has its OWN date
of birth: with one shared date, every chart matches step 3's query on the date pass, John and
Mary Smith tie, and John comes first only because he was registered first — step 3 would then
pass without ranking doing anything.

```bash
while IFS='|' read -r n dob; do
    cnode patient-register --name "$n" --birth-date "$dob" --confirm-new
done <<'SEED'
Samantha Michaelowski|1975-02-02
John Smith|1968-11-30
Mary Smith|1981-04-17
Wei Ling Chen|1990-08-09
SEED
```

**Launch WITHOUT `--patient`**, so the window opens on the front door. Once live, then once with
`--mock` (same gestures; the mock's matching rule is not db/046's, so the two figures are
reported separately and never merged):

```bash
CAIRN_CONN="$WINDOW_CONN" cargo run --release -p cairn-gui-tauri -- --key "$NODE_KEY"
cargo run --release -p cairn-gui-tauri -- --mock
```

Start the stopwatch at the first keystroke, and stop it when the identity header shows the
right name.

1. **Find an existing chart.** Type a fragment (`mich`), pick the row. Budget **≤ 5 s**.
   Paper *N* = 3 (ask details, flip drawer, pull card); architecture *M* = 2 (type, pick).
2. **Register a new patient.** In the register form, type a full name and date of birth that
   are NOT on file, answer the prompt ("None of these — register a new patient"). Budget
   **≤ 20 s**. Paper *N* = 5; architecture *M* = 4 (type fragment, read list, complete the
   form, answer the prompt); *K* = 3 when the prompt is empty.
3. **Register someone already on file.** Type `John Smith` / `1968-11-30` in the register form.
   The prompt must show that chart FIRST — it matches on the name AND the date, Mary Smith on the
   name only — then pick it ("This is them — open chart"). Record whether it was first. This is
   the wrong-duplicate case the prompt exists for. (Four seeded charts never reach the cap of
   five, so this step exercises the ranking, not the truncation; the truncation rate is the
   measured figure linked above.)

**Accessibility, same pass:** the browse list and the prompt rows announce name, age and
identity state in one utterance; each prompt row's text includes "This is them"; opening a chart
moves focus to the patient's name; a failed search is announced as a failure, never as "no
match".

Record in the template's *Front door* section. A figure outside its budget is the finding: file
it, do not adjust the budget.

## 9. Compare and link: "Same person as…" (R2b-1, [#681](https://github.com/cairn-ehr/cairn-ehr/issues/681))

The repair path's own §1.2 figure. Start with a chart already open (`--patient <uuid>`, as in
section 3) — finding the chart is measured separately, in section 8.

**Live** — the node from section 0/2, with a second chart registered that is NOT the one you
opened (a plausible duplicate — same or a near-miss name):

```bash
cnode patient-register --name "Bench Patient" --birth-date 1980-01-01 --confirm-new
cd cairn-gui
CAIRN_CONN="$WINDOW_CONN" cargo run --release -p cairn-gui-tauri -- \
    --patient "$PATIENT" \
    --key "$NODE_KEY" --key-hold-seconds 900
```

**`--mock`** (no database; the fixture's second chart is reachable by typing a fragment of its
name). Fixture mode READS everything and refuses only the WRITE, so a `--mock` run ends at the
refusal line "fixture mode: this window is showing mock data and cannot write" in the panel's
status line, never at "Linked …" — stop the stopwatch there:

```bash
cargo run -p cairn-gui-tauri -- --mock --patient 00000000-0000-0000-0000-000000000001
```

Start the stopwatch at the press of **"Same person as…"**, not before — opening the panel is
part of the gesture; finding the chart you OPEN (section 8) is not. Finding the OTHER chart
(step 2) is inside the stopwatch. Stop it when the outcome line reads
"Linked — this record now combines N charts" (live), or at the fixture-refusal line (`--mock`).

1. Press **"Same person as…"**.
2. Type part of the other chart's name into the panel's own search.
3. Press **Compare**.
4. Read the panel: veto findings (if any), then the two-column table (**This record** / **Other
   record**), then the other record's current medications.
5. Press **Link — same person**.

Budget **review-and-link ≤ 20 s**, of which the side-by-side read is the load (§1.2 in the
design page's R2b section). Record, per run:

- the wall time from step 1 to the outcome line;
- the number of veto findings shown (0 is a legitimate answer — the panel renders nothing for
  an empty list, never "no conflicts");
- whether the clinician's key was already held, or the run also measures typing the credential.

Repeat at least five times (fresh chart pairs each time — `patient-register` again), because a
single sample tells you nothing about the tail.

**Accessibility, same pass as section 6** (VoiceOver on macOS, keyboard only):

- [ ] Pressing **Compare** announces the veto findings **before** the table (the findings appear
      on Compare, not when the panel opens — nothing has been compared yet then).
- [ ] The table's two column groups are announced as **"This record"** / **"Other record"**,
      not by position.
- [ ] An absent fact reads as a word ("not recorded", or "unknown — registration not yet
      received here" for a chart not held on this node), never silence.
- [ ] The Link button is reachable by Tab and announces when it is disabled (a link in flight).
      A comparison that could not be read in full shows NO Link button at all — it is hidden,
      not disabled, and the panel names what could not be read.
- [ ] Every outcome in the panel — a refusal, the fixture refusal, a failed Compare, an Outranked
      link ("Recorded, but NOT in effect …") — is both SHOWN and announced in the panel's status
      line, never silent. After a refusal that cannot change on retry, the Link button is gone
      (compare again to get it back). No key to sign is NOT such a refusal: the button stays —
      enter the staff ID and passphrase, then press "Link — same person" again.
- [ ] Closing the panel (Esc or "Close comparison") returns focus to "Same person as…"; opening
      or switching to a different chart closes the panel, and focus goes to the new chart's
      heading (or to the front door) — not to "Same person as…".

Record in the template's *Compare and link* section. **A figure outside the ≤ 20 s budget is a
finding to file, never a budget to adjust** (§1.2's own rule, echoed here because this is the
slice that first measures it).

## 10. Unlink one link: "Review this link…" (formerly "Not the same person…"; R2b-2, [#681](https://github.com/cairn-ehr/cairn-ehr/issues/681) · [#699](https://github.com/cairn-ehr/cairn-ehr/issues/699))

The repair path's second §1.2 figure. **Live only** — fixture charts are never linked, so a `--mock`
chart has no "How these charts are linked" list and nothing to unlink; there is no `--mock` variant to
time. Set up a wrongly linked pair, then open one of its charts:

`cnode` is section 2's function (`cairn-node --conn "$CONN" --key "$NODE_KEY"`); `link-charts` takes only the
human's `--attester-key` (the top-level `--key` is already in `cnode`). Register three charts, then
chain them A–B and B–C so that the wrong link (B–C) is the one that does not touch the chart you open:

```bash
reg() { cnode patient-register --name "Bench Patient" --birth-date 1980-01-01 \
    --confirm-new | sed -n 's/^registered patient //p'; }
A=$(reg); B=$(reg); C=$(reg)
[ -n "$A" ] && [ -n "$B" ] && [ -n "$C" ] || { echo "registration failed" >&2; exit 1; }
cnode link-charts "$A" "$B" --attester-key /tmp/dr-a.key
cnode link-charts "$B" "$C" --attester-key /tmp/dr-a.key
cd cairn-gui
CAIRN_CONN="$WINDOW_CONN" cargo run --release -p cairn-gui-tauri -- \
    --patient "$A" \
    --key "$NODE_KEY" --key-hold-seconds 900
```

Open A and unlink **B–C**, the link on the list that does not touch the chart you opened. **Note:**
this setup registers all three charts locally, so the unlink is filed under a subject (B or C), NOT under
the opened chart — it does not exercise the third-chart filing of ADR-0077 (#699 (a)). That live pass —
B and C held only through a peer, the unlink filed under the opened A — is still OWED (the human live
Tauri-IPC pass). To try a cycle instead, also `link-charts "$A" "$C"` in the setup. (For a
single-link run, register only A and B and link them.) `/tmp/dr-a.key` is the human key enrolled
earlier in this runbook; substitute your own.

> [!NOTE]
> Since R6 ([#716](https://github.com/cairn-ehr/cairn-ehr/issues/716)) the line's button reads
> **"Review this link…"** — one door per line, opening the same side-by-side panel, which now offers
> every verdict that applies. The panel's **Unlink — not the same person** is unchanged, but it is now
> signed only over the pair the panel read: if another judgement about the pair was recorded while you
> were reading, it is refused ("what stands between charts … changed while you were judging it
> (another judgement about the pair was recorded meanwhile) — nothing was done"), both verdicts go,
> and the window re-reads the chart by itself (#797): the outcome line adds "The chart has been read
> again — review it before you act on it." Review the line again from the re-read list. An
> un-attested link also shows **Same person — confirm** beside it when both charts are held on this
> node; that verdict is timed in section 13, not here. If the pair's attestation changed after the
> list was drawn (a colleague confirmed it, say), the panel opens with no comparison and no verdict,
> only "this pair has been judged since the list was drawn — nothing was done", and the chart is
> re-read the same way. A change that leaves the attestation as it was is caught later instead: each
> verdict is signed only over the judgement the panel read.

Start the stopwatch at the press of **"Review this link…"** on a link's line, not before — choosing
WHICH link is wrong is the cognitive load and is inside the gesture only from the press; the panel then
lays the two charts side by side. Stop it when the outcome line reads "Unlinked — chart(s) … are no
longer part of this record" (or "Recorded that charts … are different people — but they still read as
one record through other links …" for a cycle (A–B, B–C, A–C: unlinking one edge leaves the other two joining the charts), a legitimate outcome that also stops the clock).

1. Press **"Review this link…"** on the wrong link's line.
2. Read the panel: findings (if any), then the two-column comparison of the two charts.
3. Press **Unlink — not the same person**.

Budget **review-and-unlink ≤ 15 s** (§1.2 in the design page's R2b section). Record, per run: the wall
time from step 1 to the outcome line; the number of findings shown (0 is legitimate — nothing renders,
never "no conflicts"); whether a key was already held. Repeat at least five times with fresh pairs.

**Accessibility, same pass as section 6** (VoiceOver on macOS, keyboard only):

- [ ] Each link's button is announced with its **own** text — which two charts, how the link was made
      (a clinician's judgement, or without one on record here), and when — never a bare
      "Review this link…" repeated down the list.
- [ ] Pressing the button announces the findings **before** the table.
- [ ] An absent fact reads as a word, never silence.
- [ ] Opening the unlink panel closes the link panel and vice versa — the two are never on screen
      together.
- [ ] Every outcome (refusal, Outranked, StillJoined, a link that is gone) is shown AND announced.
- [ ] Close (Esc or "Close comparison") returns focus to the link button that opened the panel; after a
      successful unlink, focus lands on the patient's heading, not `<body>`.

Record in the template's *Unlink one link* section. **A figure outside the ≤ 15 s budget is a finding
to file, never a budget to adjust.**

## 11. Review a possible duplicate from the banner (R5a, [#680](https://github.com/cairn-ehr/cairn-ehr/issues/680))

The repair path's third §1.2 figure, and the first that measures the **ambient** banner. **Live
only** — fixture mode (`--mock`) has no proposals (no identity model, #722), so a `--mock` chart never
shows a banner and there is no mock variant to time.

Setup: register two near-identical charts (the same name and date of birth, as section 10's `reg`),
give B one active medication (section 2's seeding, on B — so the banner has a drug list to show),
run the duplicate-check worker once so the pair becomes a proposal, then open chart A. `--once` drains
the queue and exits (0 = clean; 1 = a chart failed or an uncaught non-database error; 2 = a database
error — see `docs/developers/running-the-duplicate-check.md`); anything but 0 means the banner you are
about to time may be missing, so fix that first:

```bash
reg() { cnode patient-register --name "Bench Patient" --birth-date 1980-01-01 \
    --confirm-new | sed -n 's/^registered patient //p'; }
A=$(reg); B=$(reg)
[ -n "$A" ] && [ -n "$B" ] || { echo "registration failed" >&2; exit 1; }
uv run --project matcher --extra pipeline cairn-matcher watch --once --dsn "$CONN"
cd cairn-gui
CAIRN_CONN="$WINDOW_CONN" cargo run --release -p cairn-gui-tauri -- \
    --patient "$A" \
    --key "$NODE_KEY" --key-hold-seconds 900
```

**Measurement 1 — review and judge.** Start the stopwatch at the press of **Review** on the banner
entry, not before — reading the banner is part of opening the chart, not of the gesture. Stop it when
the outcome line reads "Recorded: charts … are different people. …" (**Different people**) or
"Linked — this record now combines N charts." (**Link — same person**). Time both verdicts, on fresh
pairs.

1. Press **Review** on the entry. The compare panel opens pre-filled with the other chart.
2. Read the panel: findings (if any), then the two-column table and the other record's medications.
3. Press **Different people** or **Link — same person**.

Budget **banner-to-recorded-judgement ≤ 20 s** (§1.2 in the design page's R5a section), of which the
side-by-side read is the load. Record, per run: the wall time from step 1 to the outcome line; which
verdict; the number of findings shown (0 is legitimate); whether a key was already held.
Repeat at least five times with fresh pairs.

**Measurement 2 — chart open.** Time opening chart A (stopwatch from launch or the open gesture, to
the medication table and banner painted) **with** the banner present, against the same node opening a
chart with **no** proposal (a single-chart open — e.g. a chart registered with nothing similar).
Budget: banner open **≤ single-chart open**. The banner reads eagerly (the other record's lines
included); a miss decides whether the other record's lines move to a second paint — it is a finding,
not an adjustment. Five opens each, same session state (a key held or not, the same for both).

**Check 3 — the "running" line on a checked chart (R7, [#742](https://github.com/cairn-ehr/cairn-ehr/issues/742)).**
No stopwatch: it adds no act. With chart A checked (the worker drained) and **no** worker running,
register one more chart (`C=$(reg)`) and re-open A within five minutes **of registering C**. A's
banner area reads "Duplicate check running — 1 chart waiting." although A itself is checked: a
duplicate registered after A's check can only be proposed when IT is checked, so "no banner" would
mean "not yet known". Later than five minutes the node correctly reads "Duplicate check is behind —
…; 1 chart waiting." instead (no worker ran) — a slow re-open, not a failure of this check.
Then drain once (`cairn-matcher watch --once`) and re-open A: the line is gone, and C, being a
near-duplicate, now shows as a banner entry. A whole-population re-check alone (a matcher update)
never brings this line onto a checked chart.

**Accessibility, same pass as section 6** (VoiceOver on macOS, keyboard only):

- [ ] The banner is announced as a **region** with its label, **never as an alert** — no interruption
      on chart open, no sound, and it does not re-announce when you move around the chart.
- [ ] **Focus does not jump** on chart open: it lands where it did before (the chart's heading or the
      front door), never on the banner or on **Review**.
- [ ] Each **Review** button announces **whose record** it opens (its `aria-describedby` points at the
      entry's first identity line), so two entries are told apart by name and date of birth, never a
      bare "Review" repeated.
- [ ] The rotor's **heading list** names each entry by whose record it is ("Mary SMYTHE · born … · chart …",
      or "Chart <id>" when the record's identity could not be read), never N identical headings (#737); the entry's
      status ("Possible duplicate — not yet reviewed", or accepted) is the line read right after it.
- [ ] The other record's medications are announced under "On the other record — not part of this
      one until linked", never read as lines of the open chart; a failed read is a spoken line, not
      silence.
- [ ] Pressing **Review** shows both **Different people** and **Link — same person** (when the
      comparison is complete); a comparison that could not be read in full shows **neither**. After a
      refusal that cannot change on retry both are gone (compare again to get them back).
- [ ] Every outcome is shown AND announced, never silent: **Recorded** and **Linked** on the chart's
      outcome line (the panel closes and the chart, banner included, is re-read); a refusal or "already
      been judged or resolved" in the panel's status line.
- [ ] **A pair judged elsewhere meanwhile** (#745 point 3): launch a second window on the same chart
      (the launch command above again, in a second terminal), record "Different people" there, then press **Different people** in the first. The refusal says
      "nothing was done" and **never "reload the chart"**, and the banner redraws itself without the
      pair; a stop reason you had half-typed beside a drug is still there.
- [ ] **A chart that cannot be read** (#745 points 1–2): with a chart open, pause the chart read as in
      section 14, Measurement 2 (rename `cairn_person_charts`), and press **Sign off**. The outcome line
      says "Could not read the chart"; the banner does **not** vanish and offers no **Review** on a
      stale pair — it says "Could not check for possible duplicates: …". Rename the function back and
      press **Read this chart again**: the banner is back as before. (The rename keeps the fault to
      the read; a restarted Postgres is section 14's check (c).)
- [ ] Closing the panel (Esc or "Close comparison") returns focus to the **Review** that opened it
      (to "Same person as…" if the banner has been redrawn since).

Record in the template's *Review a possible duplicate* section. **A figure outside the ≤ 20 s budget,
or a banner open slower than the single-chart open, is a finding to file, never a budget to adjust.**

## 12. Clear the possible-duplicate tray (R5b, [#680](https://github.com/cairn-ehr/cairn-ehr/issues/680))

The repair path's fourth §1.2 figure: the records clerk's tray at the front door, which gets a human
to the right chart without knowing which one to open. **Review opens the NEWER record** (usually the
just-registered duplicate); its R5a banner then shows the older one, so the judgement itself is
section 11's gesture.

**Setup.** Five near-duplicate pairs, each pair a name and a date of birth registered twice (distinct
names and dates, so pairs do not match each other), then one drain of the duplicate-check worker
(exit 0, as in section 11). For a banner with a drug list to read, give the OLDER chart of a pair one
active medication first (section 2's seeding). Launch WITHOUT `--patient`, with the human key:

```bash
while IFS='|' read -r n dob; do
    for _ in 1 2; do
        cnode patient-register --name "$n" --birth-date "$dob" --confirm-new >/dev/null \
            || echo "registration failed: $n" >&2
    done
done <<'SEED'
Avery Alder|1961-01-11
Blake Birchall|1962-02-12
Casey Cedarwood|1963-03-13
Drew Dunmore|1964-04-14
Emery Elmsworth|1965-05-15
SEED
uv run --project matcher --extra pipeline cairn-matcher watch --once --dsn "$CONN"
cd cairn-gui
CAIRN_CONN="$WINDOW_CONN" cargo run --release -p cairn-gui-tauri -- --key "$NODE_KEY" --key-hold-seconds 900
```

The front door shows **"Possible duplicates (5)"** below the registration form (more if section 11's
pairs are still open; record the figure). A status line above the tray means the node is not Current;
record it, because then an empty tray would not mean "none".

**Measurement 1 — tray to recorded judgement.** Start the stopwatch at the first act on the tray and
stop it at section 11's outcome line ("Recorded: …" or "Linked — …").

1. Expand the tray (once per session; it stays open across returns).
2. Press the entry's **Review**. The newer chart opens.
3. Press **Review** on the chart's banner. The compare panel opens.
4. Read it.
5. Press **Different people** or **Link — same person**.

Then press **Find another patient**: the judged entry must be gone and the count one lower. Run the
first entry with the tray closed (5 acts) and the rest with it already open (4 acts, steps 2–5).
Budget **≤ 25 s per entry**. Record per run: the time, which verdict, and whether the tray was open.

> [!NOTE]
> The plan's step count (4 → 3) leaves out step 3, the banner's own Review. As built (UI),
> the count is **5** with the tray closed and **4** with it open, against paper's 4. Record the
> real count; whether the tray's Review should open the compare panel directly is [#754](https://github.com/cairn-ehr/cairn-ehr/issues/754).

**Measurement 2 — find, with the tray present and with it OPEN.** Re-run section 8's step 1 (`mich`,
budget **≤ 5 s**; needs section 8's seed on this node) three ways, five runs each: no open
proposals (tray hidden); tray present and closed; tray **open**, timed right after **Find another
patient**. An open tray re-reads its list on every return, over the connection the search shares.
All three must be inside ≤ 5 s, and the open-tray figure must not be visibly slower than the hidden
one. Record how many entries the tray showed.

**`--mock` variant.** `--mock` has one fixture entry, under the status line "Duplicate check has
never run on this node." Its Review opens the fixture chart with no banner (the mock has no
proposals, #722). Time only steps 1–2 there (expand → chart open), and keyboard-walk it. Report it
separately from the live figure; never merge the two.

**Accessibility, same pass as section 6** (VoiceOver, keyboard only):

- [ ] The tray's summary is announced with its count ("Possible duplicates (5)").
- [ ] It expands and collapses from the keyboard (Tab to the summary, then Space or Enter), with no
      mouse.
- [ ] Each **Review** announces the newer record's first identity line, so two entries are told
      apart by name, never a bare "Review" repeated.
- [ ] The rotor's **heading list** names each entry by BOTH its records ("Mary SMYTHE · 74 y · chart …
      — possibly the same person as Mary SMITH · 75 y · chart …"; "Chart <id>" for a side that could
      not be read), never N identical headings (#796); the entry's status is the line read right after
      it. A side with linked charts ("· and 1 linked chart") is never heard as linked to the OTHER side.
- [ ] Nothing in the tray takes focus by itself or is announced as an alert, on boot or on a return.
- [ ] On a non-Current node the status line is read before the tray, so "(0)" is never heard as
      "checked, none".

Record in the template's *Clear the possible-duplicate tray* section. **A figure outside its budget,
an open-tray find slower than the hidden one, or a step count above paper's is a finding to file,
never a budget to adjust.**

## 13. Confirm a doubted link (R6, [#716](https://github.com/cairn-ehr/cairn-ehr/issues/716) · [#723](https://github.com/cairn-ehr/cairn-ehr/issues/723))

The repair path's fifth §1.2 figure: the clinician vouches for a link the record holds but nobody has
judged, so the sign-off hold (R1b) lifts. **Live only** — fixture charts are never linked (#722), so a
`--mock` chart has no "How these charts are linked" list and no front-door doubted row.

**Setup A — a doubted machine link.** Two charts joined by an **un-attested** link (the matcher's)
that this node's hard identity check now finds a clash on (db/054's `clash`). Make the link with
section 11's pair: one drain of the duplicate-check worker, then `cnode apply-auto-candidates` (the owner's `$CONN`, not the window's `$WINDOW_CONN`: the command's own help says why). Its
summary line must read `auto-apply: applied 1 …`; a pair the worker scored for human review is not
linked. The clash itself needs a **verified** (provenance rank ≥ 60) date of birth or sex at birth that
differs between the two charts, or disjoint normalized identifiers in one system (db/016's hard veto) —
a vetoed pair is never auto-linked, so the clash must arrive AFTER the link. There is no one-line recipe
for that here; whatever you use, **check the setup before timing it**:

```bash
psql "$CONN" -Atc "SELECT low, high, kind FROM cairn_chart_set_doubts(ARRAY['$A','$B']::uuid[])"
```

It must print one `clash` row for the pair. If you cannot produce a clash, time the same gesture on an
**undoubted** machine link (every un-attested link offers the confirm) and record the run as
"no clash" — the side-by-side read of the clash is the load this budget is about, so never merge
those figures with the doubted ones.

**Setup B — a bridged separation.** Three charts linked by a clinician A–B, B–C and A–C, then a
clinician's "different people" on A–C: the record still joins A and C through B (StillJoined).

```bash
reg() { cnode patient-register --name "Bench Patient" --birth-date 1980-01-01 \
    --confirm-new | sed -n 's/^registered patient //p'; }
A=$(reg); B=$(reg); C=$(reg)
[ -n "$A" ] && [ -n "$B" ] && [ -n "$C" ] || { echo "registration failed" >&2; exit 1; }
cnode link-charts "$A" "$B" --attester-key /tmp/dr-a.key
cnode link-charts "$B" "$C" --attester-key /tmp/dr-a.key
cnode link-charts "$A" "$C" --attester-key /tmp/dr-a.key
cnode unlink-charts "$A" "$C" --attester-key /tmp/dr-a.key
cd cairn-gui
CAIRN_CONN="$WINDOW_CONN" cargo run --release -p cairn-gui-tauri -- \
    --patient "$A" \
    --key "$NODE_KEY" --key-hold-seconds 900
```

`cairn_chart_set_doubts` over the three charts must print one `separated` row for A–C.

**Measurement 1 — confirm a doubted link (Setup A).** Open one chart of the pair. Its "How these
charts are linked" line ends "— in doubt: this node's identity check finds a clash between these two
charts; while this link is in doubt, only lines recorded on this chart alone can be signed off". Start
the stopwatch at the press of **"Review this link…"** on that line — reading the mark is part of
opening the chart; choosing to act on it starts the gesture. Stop it at the outcome line
"Confirmed — charts … and … are recorded as the same person by your judgement."

1. Press **"Review this link…"** on the doubted line.
2. Read the panel: the findings (on Setup A the clash is one of them), then the two-column comparison.
3. Press **Same person — confirm**.

After the chart re-reads, the line has lost its mark and reads "linked by a clinician's judgement";
the medication list's doubted-link note is gone when no other link of the record is in doubt.

**Measurement 2 — re-link a bridged separation (Setup B).** The list carries a line "{A} and {C} — a
clinician recorded on {date} (UTC) that these are different people, but other links still join them".
Start at the press of **"Review this judgement…"** on it; stop at "Linked — your judgement that charts
… and … are the same person now stands over the earlier judgement that they were different people;
both stay on record."

1. Press **"Review this judgement…"**.
2. Read the panel: the comparison, and the note naming the earlier judgement and its date.
3. Press **Same person — link them**.

**No key to sign.** Run one pass with no key held and no credential typed: a press signs nothing
(the panel's status line says ""…" was not signed. Enter your staff ID and passphrase to sign."; the
button stays). Enter the credential and press again. Do not time that pass.

Budget **review-and-confirm ≤ 15 s** for both measurements (§1.2 in the design page's R6 section; the
same budget as section 10's unlink). Steps: confirm paper 2 → 2 acts; bridge paper 3 → 2 acts. Record,
per run: the wall time from step 1 to the outcome line; which measurement; the number of findings shown
(0 is legitimate on an undoubted link, never on Setup A); whether a key was already held. Repeat
at least five times with fresh pairs. An outcome of "Recorded, but NOT in effect: …" (Outranked)
means a colleague's judgement landed meanwhile — record it, and both verdicts must then be gone from
the panel.

**Measurement 3 — the front door (#723).** With a Setup A pair still doubted, re-run section 8's
find (≤ 5 s) for "Bench Patient". The person row reads **"Linked charts — a link between them is in
doubt (N charts)"**, never "One person — N linked charts"; after Measurement 1 confirms the link, the
same find reads "One person — 2 linked charts". The label adds no act, so the budget is section 8's.
In the CLI's `patient-search`, a doubted row's further charts are prefixed "↳ in doubt: " (an
undoubted row keeps "↳ linked: ").

**Check 4 — an unverified link (R7, [#743](https://github.com/cairn-ehr/cairn-ehr/issues/743)).**
No stopwatch: it adds no act (acting on it is Measurement 1's gesture). Setup: an **un-attested**
link (as Setup A) between two charts whose identifiers in ONE system differ while at least one side
carries no normalized form (no format profile — e.g. `00123` vs `123`: db/016's `degrade_hold`).
Registering the two charts with differing `--identifier` values does NOT build this: Setup A's
`apply-auto-candidates` refuses an auto-link on ANY veto, this soft one included, so the differing
identifiers must arrive AFTER the link — and there is no CLI recipe for an identifier assertion on
an existing chart here. Whatever you use, check the setup before looking:

```bash
psql "$CONN" -Atc "SELECT low, high, system FROM cairn_chart_set_unverified_links(ARRAY['$A','$B']::uuid[])"
```

It must print one row naming the system. The link's line then ends "— unverified here: the two
charts hold different {system} identifiers, and this node cannot tell a real mismatch from a
difference in formatting"; it is NOT marked "in doubt", and the medication list holds nothing back
because of it (db/016: a soft veto never demotes a link). "Review this link…" opens the usual panel;
after **Same person — confirm** the note is gone.

**Accessibility, same pass as section 6** (VoiceOver on macOS, keyboard only):

- [ ] The doubted row's label is read aloud as "Linked charts — a link between them is in doubt
      (N charts)".
- [ ] The doubted line's button is announced with its own line text (which two charts, how linked,
      and the doubt), never a bare "Review this link…" repeated.
- [ ] From the panel, **Tab reaches both verdicts** (Same person — confirm, and Unlink — not the same
      person) with no mouse; on a separated line, Tab reaches **Same person — link them**.
- [ ] While a verdict is in flight both verdicts are disabled; once one lands both are gone
      (Outranked included).
- [ ] **Escape closes the panel** and returns focus to the "Review…" button that opened it; after a
      confirm, focus lands on the patient's heading.
- [ ] Every outcome (Confirmed, Linked, Outranked, a refusal, "changed while you were judging it") is
      shown AND announced; the panel is never an alert and never takes focus on its own.

There is no *Confirm a doubted link* section in `TEMPLATE.md` yet; record this run in its own block
after the *Unlink one link* section, under the same headings. **A figure outside the ≤ 15 s budget, a
step count above paper's, or a doubted row read as "One person" is a finding to file, never a budget to
adjust.**

---

## 14. A colleague's link lands mid-review (#797)

The "the record changed under you" gesture: the clinician holds chart A, a colleague links A to B, and the
clinician's next write is refused. The window must re-read the chart itself and say what changed. **Live
only** (a `--mock` chart never changes under you). Paper counterpart: you write up a drug chart and a
colleague has meanwhile written a new one; the two are reconciled.

**Setup.** Two charts A and B, registered with section 13's `reg()` helper and **not** linked, at least one
unsigned line on A, the window open on A with dr-a's key held. A second node, or `cairn-node` on this same
node as another clinician, is ready to run `cnode link-charts "$A" "$B" --attester-key <the other
clinician's key>` (`cnode` as in section 13). Check the window shows no "How these charts are linked" line before you start.

**Measurement 1 — the refusal and the re-read.** Start the stopwatch at the press of **Sign off** (step 3);
stop it when the outcome line has finished saying "Now part of it: B.".

1. Open chart A.
2. From the second node, link A to B as the other clinician.
3. Press **Sign off** on A.
4. Read the outcome line: the refusal ("Sign-off failed: the linked charts changed while this list was on
   screen — nothing was done."), then "The chart has been read again — review it before you act on it.",
   then "This record changed while it was on screen. Now part of it: <B's identity line>." The list now shows B's lines and the link line.
5. Press **Sign off** again; it succeeds.

**Measurement 2 — Read this chart again.** (a) A read that fails and then recovers, while the node keeps
running: the fault is injected into the read itself. The chart read starts with `cairn_person_charts` (the record's chart
set; `cairn_node::patient::person::person_charts`), resolved by name on every read. With the chart open and
its list drawn, in a second terminal.

> [!WARNING]
> **Only on a throwaway measurement rig — never on a node anyone is charting on.** While the function is
> renamed, every chart read and write on that node fails.

```bash
psql "$CONN" -c "ALTER FUNCTION public.cairn_person_charts(uuid) RENAME TO cairn_person_charts_paused"
```

Press **Sign off**. Expect "Sign-off failed: …" then "Could not read the chart: …", both naming the missing
`cairn_person_charts(uuid)`; the rows stay and nothing is signed. Then:

```bash
psql "$CONN" -c "ALTER FUNCTION public.cairn_person_charts_paused(uuid) RENAME TO cairn_person_charts"
```

Press **Read this chart again**; time it from the press to the list drawn (1 act). The outcome line drops
"Could not read the chart: …" and keeps "Sign-off failed: …". (Pressed BEFORE the rename back, it fails
again and replaces the failure with the new one — it never adds a second.) (Why not `REVOKE`: `$CONN` connects as `$USER`, the cluster's
superuser on this rig, and a superuser ignores grants.) **Undo it before anything else.** Restarting the
window or the node does NOT heal a forgotten rename: a connect replays the schema only when its stamp has
changed (ADR-0080), so normally nothing recreates the function. The rename back is the remedy. If a schema
replay did run meanwhile (a cleared stamp, a different build, `cairn-sync init`), it recreated
`cairn_person_charts` and the rename back is refused because the name exists; then drop the copy instead:
`psql "$CONN" -c "DROP FUNCTION public.cairn_person_charts_paused(uuid)"`.

(b) The "This list is out of date …" notice with Sign off disabled fires only when a stale refusal succeeded and the re-read right after it failed, a one-round-trip
window: it is observable only with fault injection or the headless mock walk, never by hand. Do not look
for it in this pass.

(c) A lost CONNECTION (#807). The window reopens its one connection when it is lost. With the chart open,
restart the node's Postgres (`"$PGBIN/pg_ctl" restart -D "$MDATA"`, section 0's cluster), then
press **Read this chart again**: the list draws, with no window restart. The window's terminal shows "the
database connection was lost; reopened it". Pressed while Postgres is still down, it fails and the screen
says why: "…could not be reopened: connecting to the database: …" with the cause (#876). A press within
two seconds of that tries nothing and says so ("reopening it failed 0.8 s ago, so this press tried
nothing (press again in 1.2 s to retry)"); a later press tries again. Nothing retries on its own. A list that never comes back without a window restart is a finding, and so is a
failure that says only "connection closed". A node that ANSWERS and refuses (#880): restart Postgres,
and before pressing, rename the window's database from another one, `psql -d postgres -c 'ALTER DATABASE
<db> RENAME TO <db>_away'` (the window's reopen is then refused whatever the auth method, a superuser's
included, unlike a `REVOKE CONNECT`; stop any other node process on that database first, or the rename
is refused). Press **Read this chart again**: the screen names `[3D000]` and says "pressing again fails
the same way until an operator acts"; a press within two seconds says the same. (That button is always
shown, so it is not the retry to look for.) Press **Sign off** (or Stop on a line): it says "Nothing was
done: the connection …", names `[3D000]` and says "until an operator acts" — a write that says "press
again in … s to retry" here is a finding. Close the chart and search at the front door: it says "The
search FAILED — this is NOT a "no match"", names `[3D000]`, says "until an operator acts" once (#885), and
says "Try again" nowhere. Rename it back (`… RENAME TO <db>`), wait two seconds (a press inside the backoff
tries nothing and repeats the refusal), and search again: the results draw. A front-door search failure
that reads like an empty result, or says "Try again", is a finding. Then the front door's top line (#882):
fill the register form until its search result shows, restart Postgres, rename the database away again as
above, and press **Register**. It refuses, the line at the top of the front door says an operator must
act, and neither line says "until an operator acts" twice (#885). Rename it back, wait two seconds, and
search: the results draw and the top line clears. A top line still warning after the results draw is a
finding — on an enrolled node; an unenrolled one rightly keeps its enrolment warning, so enrol the rig's
node first.

(d) A command the node does not answer (#875 option 2). With the chart open, press **Read this chart again**:
no bar appears (an answer inside 2 s shows nothing — a bar that flashes on an ordinary press is a finding). Then,
from `psql` on the window's database, `BEGIN; LOCK TABLE medication_statement IN ACCESS EXCLUSIVE MODE;` and
leave it open (every chart read reads that table, through `medication_thread_group`). Press **Read this chart
again**: after about 2 s a bar at the top of the page reads "Waiting for the node…" and empties; at 15 s from the
press it reads "Reconnecting…", and the outcome line then says the read failed, ending in "connection closed". In
`psql`, `ROLLBACK;`, then press again: the list draws and the bar is gone. Record the time from the press to the
failed-read line (budget ≤ 15 s). A press made while another was already waiting gets its own 15 s once the
first is cut off, since the lock is still held: up to about 35 s from the first press. If a second machine is to
hand, repeat over the LAN to a node there, pulling its cable instead of taking the lock; there a queued press
fails once the reopen does (≤ 20 s). A window that stays frozen past those, a bar that never leaves, or a bar
announced more than twice by VoiceOver ("Waiting for the node…", then "Reconnecting…") is a finding.

**Check 3 — your OWN link is never announced.** Press **Same person as…** on A, compare with B, and link
them yourself. The outcome line reads "Linked — …" and nothing says "Now part of it". Then repeat with a
colleague's link landing at the same moment if you can; only the colleague's change may be named, and then
"The chart has been read again — review it before you act on it." comes before it (#811). (The wording after an
interrupted Link or Unlink, "If it was recorded, a change the re-read names may be your own link.", needs the connection to drop during
the commit, so it cannot be produced by hand. Do not look for it in this pass.)

Budget: **re-read and sentence ≤ the single-chart open time (section 8 / section 11) after the press**;
**Read this chart again ≤ that same open time, 1 act**. Steps: paper N = 3 (write the chart, be told by a colleague or nurse, reconcile; commonly
the discrepancy is never noticed at all) → forced M = 3 (the refused press, review the re-read list, press
again) → bundling target K = 3. Failed read: today 3 acts (close, find, reopen) → 1. Record, per run: the wall time from step 3 to the end of the sentence; whether the sentence named
exactly the colleague's chart. Repeat at least five times.
**A figure above the budget, a sentence naming your own link, a sentence before the list drew, or a list that
vanished when the read failed is a finding to file, never a budget to adjust.**

**Accessibility, same pass as section 6** (VoiceOver on macOS, keyboard only):

- [ ] The refusal, "The chart has been read again …" and "This record changed … Now part of it: B." are announced in
      that order.
- [ ] "Read this chart again" is reachable by Tab at all times (also with no failure), and announced by that label.
- [ ] Focus stays where the clinician put it; nothing takes focus on its own.

`TEMPLATE.md` carries a matching block (*A colleague's link lands mid-review*).

---

## 15. Withhold and release a drug (#832, ADR-0081 slice B)

The window's two new gestures: **Withhold** (the paper chart's "W", a time-bounded hold on a line that stays
current, signable and ceasable) and **Release** (ending a hold early). Paper counterpart: writing W in the
drug chart's administration slot(s) with initials and the reason in the slot code or a short note; rescinding is
a note; resuming is giving the next dose, no act. **Live only for the timings and the behaviour checks** (the
write is the measurement, and a `--mock` window refuses every press as "fixture mode" before reading the time);
the `--mock` window is enough for the accessibility checks below.

**Budgets (from the slice plan's §1.2).** Withhold **≤ 10 s from the chart open to recorded** (paper *N* = 2 →
architecture-forced *M* = 2 → UI bundling target *K* = 2: type the reason in the line's W field, press
*Withhold*. The wire needs an explicit end; the window bundles it — "until reviewed" is preselected on screen — so
no further act). A W *until a time* adds the time entry, as
paper's slot choice does. Release **≤ 5 s** (paper 1 → Cairn 1; its reason is optional). Resume is paper 0 →
Cairn 0: expiry is a read-time fold and writes no event. **A figure outside its budget, or `M > N`, is the
finding — file an issue; never adjust the budget.**

**Setup.** Section 0–3 (a throwaway database, enrolled clinician, window launched with `--patient`). The
chart needs at least two **current** drugs; section 2's five-drug chart is fine, or, minimal (run 2026-10-21
against a fresh database, same shape as sections 0–2):

```bash
cnode() { cairn-node --conn "$CONN" --key "$NODE_KEY" "$@"; }   # a function: an unquoted $CONN would word-split
PATIENT=$(cnode patient-register --name "Bench Patient" --birth-date 1980-01-01 \
    --confirm-new | sed -n 's/^registered patient //p')
cnode medication-assert "$PATIENT" --term atorvastatin --dose-amount 40 --dose-unit mg --formulation tablet
cnode medication-assert "$PATIENT" --term metformin --dose-amount 1 --dose-unit g --formulation tablet
cnode medication-list "$PATIENT"     # two lines, both [current] — unsigned
```

**Measurement 1 — Withhold.** Start the stopwatch when the window appears (as section 4). Enter the staff ID and
passphrase (unless a key is held), type a reason into the W field of one current line, leave *until* on "until reviewed", press *Withhold*. Stop when the
line draws its W ("W until reviewed"). Repeat with *until a time* and a time later today (stop at "W until
HH:MM"). Five runs each; repeated holds on one line are admitted, so a fresh chart is not needed per run.

**Measurement 2 — Release.** On a line that now shows a W, start the stopwatch, press *Release* (reason optional).
Stop when the W is gone from the line. Five runs.

**Behaviours to confirm while you are there** (each is a pinned guard; a miss is a finding):

- [ ] The W reads in **local time** ("W until 20:00"; "W from Thu 22 Oct 08:00" for a hold not yet started; a
      year only when it is not this year).
- [ ] Choosing *until a time* and leaving the time empty, or typing a **partial** time, is refused with a
      message; it is never recorded as "until reviewed". Typing a time alone switches the select to
      "until a time"; setting the select back to "until reviewed" empties the time field (a partly typed
      time too — WebKit needs the type reset in holds.js), and the W is then
      recorded "until reviewed" (never until the time that was typed).
- [ ] A second press during the write does nothing (the pressed button is disabled until the re-read). After
      a recorded W the line's W fields are empty; after a refusal they keep what you typed and the button
      works again — unless the refusal found the list out of date, when it stays down until the re-read draws
      the line afresh.
- [ ] A W *until a time* two minutes ahead: leave the window alone; within a few seconds of that minute the W is
      gone from the line and the status word reads "current" again, with no press. A W written from the CLI with
      `--from` two minutes ahead reads "W from HH:MM" and "current", then "withheld" at that minute.
- [ ] The line still counts in the *Sign off N unsigned* button and still has *Stop*.
- [ ] Hovering the W does not make it flicker (the detail must not move the table).
- [ ] A line that is both withheld and about to be signed shows both left bars (will-sign outermost).
- [ ] A Release while a colleague's newer W is on the line but not yet drawn (write one from the CLI after the
      window drew the line) leaves that W on the line, and the outcome line says so ("… added since this line was
      drawn are still on this line …"), even when every W the line drew had already gone ("… no longer on this
      line — nothing was done. 1 hold(s) added …", #839).
- [ ] On a line flagged as possibly another person's drug that has no record on the opened chart (§13's
      doubted-link set-up, opened from the OTHER chart), neither *Withhold* nor *Stop* is drawn, and the line
      says "… stop or withhold it from its own chart" (#841).

**Read it back (the same §5 query, plus the events).** The write half is aggregated with no user, patient or
timestamp, by gesture kind `withhold` and `release`:

```bash
psql "$CONN" -c "SELECT * FROM ui_gesture_timing ORDER BY gesture_kind, size_bucket"
```

Expect `withhold` and `release` rows beside `signoff` and `cease` once the window has done them (a fresh
database shows `(0 rows)` until then; the check constraint admits all four kinds). The record itself:

```bash
psql "$CONN" -At -c "SELECT event_type, count(*) FROM event_log
                     WHERE event_type LIKE 'clinical.medication-withhold%' GROUP BY 1"
cnode medication-list "$PATIENT"
```

Expect one `clinical.medication-withhold.asserted` per press of *Withhold* and one
`clinical.medication-withhold-release.asserted` per thread released. While a hold is live `medication-list`
prints under its line `    W withheld until reviewed: <reason> (by <kid>) [withhold <event id> on thread <id>]`;
after a release the line is bare again. To exercise the CLI half without the window (this is how the commands
above were checked): `cnode medication-withhold "$PATIENT" <thread> --reason "NBM" --until reviewed`, then
`cnode medication-release "$PATIENT" <thread> --withhold <event id>`.

**Accessibility, same pass as section 6** (VoiceOver on macOS, keyboard only; `--mock` is enough):

- [ ] Tab reaches each W; VoiceOver reads its visible text ("W until 20:00") and then the reason ("Withheld
      until … by <key>: <reason>").
- [ ] The *until* select and the time field are each labelled with the drug.
- [ ] *Withhold* and *Release* are named by drug, not a bare "Withhold".
- [ ] The reason field of the W announces which drug it belongs to.
- [ ] A ceased line whose cessation gave a reason has its status word as a button; its reason
      (`Stopped: <reason>`) reads on focus. A cessation with no reason leaves the word as plain text.
- [ ] A **current** line with a stopped member thread that gave a reason (the fixture's metformin, #861) has
      its status word as a button too; it reads `A stopped record of this drug gave: <reason>` on focus — never
      "Stopped".
- [ ] A pinned reason (click or Enter on a W) closes on **Escape**, and on a click elsewhere.
- [ ] A withheld line is identifiable without colour (the W text itself).

**What could not be run here.** The stopwatch figures, the VoiceOver checks and every `ui_gesture_timing` row
need the window and a human; this section's commands were run against a throwaway database for the CLI half only.
`TEMPLATE.md` carries a matching block (*Withhold and release*).

---

## 16. Sign in and sign (GUI sign-in slice 1, ADR-0083)

The slice's gestures: the **terminal lock** (gatekeeping: may anyone use this terminal?), the **signer strip**
(attribution: who signs this act?), **Enrol staff** (a superuser vouches for a new member) and the **key hold**.
Paper counterpart: the bedside chart. Anyone on staff reads it, each entry carries its writer's own signature, and
the next nurse picks up the pen with no ceremony. **Live only** (a `--mock` window has no vault: the lock screen
is not shown and every write is refused before it asks); `--mock` is enough for the strip's labels and Tab order.

**Budgets (design §7, the slice plan's §1.2).** Sign **≤ 3 s, credential included** (a pen signature is about
2 s). The strip is typed BEFORE the press, so the stopwatch starts at the **first keystroke in Staff ID** and stops
when the outcome line reports the result; record the press → outcome share beside it. Steps: paper *N* = 1 (a
signature) → architecture-forced *M* = 1 (the write gesture carries staff ID + passphrase) → UI bundling target
*K* = 1. **The next nurse: 0 extra acts** (paper 0 → Cairn 0: nothing is logged in to switch away from). The
terminal unlock is a gatekeeping act with no per-write cost. **A figure outside its budget, or `M > N`, is the
finding — file an issue; never adjust the budget.**

**Setup.** Sections 0 and 2 (a throwaway database with `cairn_pgx` **0.4.0** — db/061 refuses to load without
`cairn_vault_unseal` — and a seeded chart). Then, at the node's console, with the **owner** connection (the
console verbs were run against a fresh database on 2026-10-31; each passphrase is prompted twice with no echo, or
read for a script from `CAIRN_VAULT_PASSPHRASE` (the superuser's) and `CAIRN_TERMINAL_PASSPHRASE` (the terminal's).
They are two variables on purpose: one exported for both would make the shared terminal passphrase the superuser's):

```bash
cnode() { cairn-node --conn "$CONN" --key "$NODE_KEY" "$@"; }   # a function: an unquoted $CONN would word-split
cnode vault-bootstrap-superuser --staff-id boss --display-name "Site Admin" --email it@example.invalid
cnode vault-set-terminal-passphrase
psql "$CONN" -At -c "SELECT staff_id, role, enrolled_by FROM vault_member"   # boss|superuser|console
```

> [!WARNING]
> **Statement logging can write a passphrase, or a sealed key, into the server log.** The signing passphrase is a
> bind parameter of `cairn_vault_release`, and the sealed key blob one of `cairn_vault_enrol`/`cairn_vault_bootstrap`
> (which raise on every refusal). PostgreSQL prints bind parameters with a logged statement (`log_statement = 'all'`,
> `log_min_duration_statement` ≥ 0, `log_min_duration_sample` ≥ 0 with `log_statement_sample_rate` > 0, or
> `log_transaction_sample_rate` > 0, unless `log_parameter_max_length = 0`) and on an error
> (`log_parameter_max_length_on_error` other than 0). Check before every run, and on every real node:
>
> ```bash
> psql "$CONN" -c "SELECT name, setting FROM pg_settings WHERE name IN ('log_statement',
>   'log_min_duration_statement','log_min_duration_sample','log_statement_sample_rate',
>   'log_transaction_sample_rate','log_parameter_max_length','log_parameter_max_length_on_error') ORDER BY name"
> psql "$CONN" -c "SELECT * FROM pg_db_role_setting"   # per-role / per-database overrides
> ```
>
> Safe is `log_statement = none` (or `ddl`/`mod`: the release is a `SELECT`), `log_min_duration_statement = -1`,
> `log_min_duration_sample = -1`, `log_transaction_sample_rate = 0` and `log_parameter_max_length_on_error = 0` —
> PostgreSQL's defaults. A node that needs statement logging for
> other roles pins it off for the role the window connects as: `ALTER ROLE <role> SET log_statement = 'none'`,
> `… SET log_min_duration_statement = -1`, `… SET log_min_duration_sample = -1`,
> `… SET log_transaction_sample_rate = 0`, `… SET log_parameter_max_length_on_error = 0`.

> [!WARNING]
> **The window's connection is not encrypted** (ADR-0083 decision 3). A window on the node's own machine, connected
> over loopback (`host=127.0.0.1`, as in section 0) or a Unix socket, is fine. A window on another machine sends the passphrase and receives the signing key
> in clear. Since ADR-0084 the window refuses any database that is not on its own machine, so measure only on the
> node's machine, or through a tunnel that ends at `localhost`.

Launch the window as section 3, with **no** `--key-hold-seconds` (the default 0, a shared terminal) unless a step
says otherwise.

**Measurement 1 — the terminal lock.** The lock screen shows at launch, with focus in *Terminal passphrase*.

- [ ] A wrong passphrase: "That passphrase is not right." The field empties; the page behind stays unreachable
      by Tab.
- [ ] The right passphrase: the page shows and focus moves to *Staff ID*.
- [ ] On a node with no terminal passphrase set (skip `vault-set-terminal-passphrase` on a second rig): "No
      terminal passphrase is set. Run cairn-node vault-set-terminal-passphrase on the node."
- [ ] With the node's Postgres stopped: "The terminal could not be unlocked. Try again." — never "not right".

**Measurement 2 — enrol two nurses from the window.** Open *Enrol staff*. Type `boss` and its passphrase into the
signer strip; the new member types their own passphrase twice. Enrol `nurse-a` and `nurse-b` (role *nurse*, a
phone or an email). Time each from opening the form to "… (staff ID nurse-a) is enrolled as nurse." (no budget
yet — record it). Then confirm:

- [ ] Both passphrase fields and the strip's passphrase are empty after every attempt, success or refusal.
- [ ] Two different new passphrases: "The two new passphrases do not match. Nobody was enrolled."
- [ ] `nurse-a` again: "That staff ID is already in use. Nobody was enrolled." (asked before the superuser's key
      is released, so it costs boss no attempt).
- [ ] `nurse-a`'s own credential in the strip: "Only a superuser can enrol staff. …"
- [ ] After a successful enrol, no "Signed in for the next act" line ever names `boss`, whatever the key hold
      (repeat once at `--key-hold-seconds 900`): a superuser's key is never held.

**Measurement 3 — sign one act (the ≤ 3 s budget).** On the section 2 chart: Tab to *Staff ID*, type `nurse-a`,
Tab, type the passphrase, Tab to *Sign off* (or type a reason and Tab to a line's *Stop*), press it. Start at the
first keystroke in Staff ID; stop at the outcome line. Five runs (re-seed or Stop a different line each run).
Then:

- [ ] The passphrase field is empty after the act; with a key hold of 0 the Staff ID is empty too.
- [ ] `cnode medication-list "$PATIENT"` shows the act signed by nurse-a's key
      (`psql "$CONN" -At -c "SELECT staff_id, left(signing_key_id, 8) FROM vault_member"` maps key to person).
- [ ] `boss` and its passphrase on a clinical write: "A superuser cannot sign a clinical act. Enter your own staff
      ID." Nothing is signed.
- [ ] ` NURSE-A ` (capitals, spaces) signs as nurse-a: staff IDs are compared without case or surrounding spaces.

**Measurement 4 — the next nurse (0 extra acts).** Straight after nurse-a's act, nurse-b steps up, types her own
ID and passphrase and signs. Count every act she makes beyond those of Measurement 3 (target 0: no log-out, no
log-in). Then relaunch with `--key-hold-seconds 900` (a personal workstation) and repeat:

- [ ] nurse-a signs once; "Signed in for the next act: nurse-a." shows and the *Lock* button appears; her Staff ID
      stays in the strip and her next act needs no passphrase.
- [ ] nurse-b types only her Staff ID (no passphrase) and presses: "Enter your staff ID and passphrase to sign.";
      the "Signed in …" line clears (nurse-a's key is forgotten).
- [ ] nurse-b types her own credential: the act is signed by nurse-b (check with `medication-list`), and the line
      then names nurse-b.
- [ ] *Lock*: the line clears; the next act with an empty strip says "Enter your staff ID and passphrase to sign."
- [ ] The hold runs out: sign as nurse-a with `--key-hold-seconds 60`, wait past a minute with no act, and the
      "Signed in …" line goes by itself within about 10 s (the lock poll); the next empty-strip act asks for a
      credential.

**Measurement 5 — the back-off, read aloud.** With VoiceOver on, sign as nurse-a with a wrong passphrase, four
times. Tries 1–3 say "… That passphrase is not right." Try 4 says "That passphrase is not right. Wait 1 s, then
try again." A press inside that wait says "Too many tries. Wait 1 s, then try again." and does not count as a
failure. Then the right passphrase, after the wait, signs. Confirm:

- [ ] VoiceOver reads each sentence once, whole, from the outcome line, and the wait is said in whole seconds.
- [ ] No sentence, on screen or spoken, contains the passphrase.
- [ ] Record whether the Staff ID had to be retyped after each refusal (on a hold of 0 it is cleared with the
      passphrase): every retype is a paper-parity cost to note.

**Keyboard and screen reader, same pass as section 6** (VoiceOver on macOS, keyboard only):

- [ ] The lock screen, the strip and the enrol form are reachable and complete by keyboard alone.
- [ ] *Staff ID*, *Passphrase*, *Terminal passphrase* and each enrol field are announced by their labels.
- [ ] Enter in the strip does not submit anything (it has no submit of its own); the act is the write button.
- [ ] "Signed in for the next act: …" is announced when it appears, and nothing takes focus on its own.
- [ ] The hint: typing a Staff ID and leaving the field shows "Your hint: …" for a member who set one.

**What could not be run here.** The window gestures, the stopwatch figures and the VoiceOver pass need the window
and a human. The console verbs and the release door's answers (`wrong`, `refused-purpose`) were run against a
throwaway database on 2026-10-31. `TEMPLATE.md` carries a matching block (*Sign in and sign*).
