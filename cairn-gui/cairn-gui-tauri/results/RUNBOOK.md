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
into (pgrx's `pg18` entry in `~/.pgrx/config.toml`), so `CREATE EXTENSION cairn_pgx` works in the new cluster too.
Not the `pg_config` on `PATH`, which may be another major version (on the maintainer's Mac it is PG 16). The `init` and later
`cairn-node` flags were run end-to-end on 2026-08-03 against a `trust` rig; the cluster steps (`initdb` to `start`)
and the window's own login (below) were written on 2026-11-01 for #898/#899 and have not yet been run end-to-end,
so a first run that needs a change is a finding:

```bash
# The PG18 install cairn_pgx is installed into: pgrx's pg18 pg_config, never the one on PATH.
export PGBIN="$("$(sed -n 's/^pg18 = "\(.*\)"$/\1/p' ~/.pgrx/config.toml)" --bindir)"
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
   password and export it for the step (otherwise it is prompted for, twice). Like the owner's, it goes into a
   connection string, so no spaces or quotes. Run the `cairn-node` built from THIS checkout: the window refuses a
   database stamped by any other build ("prepared by a different build"), and a `cairn-node` on `PATH` from an
   older checkout is exactly that:

   ```bash
   export CAIRN_WINDOW_DB_PASSWORD='…'     # no spaces or quotes
   cargo run -p cairn-node --release -- --conn "$CONN" provision-window-role --role cairn_window_login
   export WINDOW_CONN="host=127.0.0.1 port=$MPORT user=cairn_window_login password=$CAIRN_WINDOW_DB_PASSWORD dbname=cairn_measure"
   ```

   Re-run it after every schema-changing build: a window refuses to start on a database its build has not been
   prepared for, and names this command.
3. **Every window launch below uses `"$WINDOW_CONN"`**; every `cairn-node` and `psql` step keeps the owner `"$CONN"`.
   The window gets it as `CAIRN_CONN="$WINDOW_CONN" cargo run …`, not `--conn`: the string carries a password, and a
   command-line argument is visible to every account on the machine (`ps`), an environment variable only to yours (and root).
   `cairn-node` reads `CAIRN_CONN` too. This rig's own `cairn-node` and `psql "$CONN"` lines pass the owner's string
   as an argument for brevity, which is acceptable only on a machine no one else logs into; on a shared machine,
   drop `--conn "$CONN"` and give `cairn-node` `CAIRN_CONN="$CONN"` in its environment instead, and give `psql`
   `-h 127.0.0.1 -p "$MPORT" -d cairn_measure` (it already reads `PGPASSWORD`).
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
and before pressing, rename the window's database from another one, `psql -h 127.0.0.1 -p "$MPORT" -d postgres -c 'ALTER DATABASE
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

## 17. Enrol in person, re-key, change a passphrase, confirm (GUI sign-in slice 2, ADR-0085)

The slice's gestures: **Enrol staff** approved by an *enroller* (a member a superuser granted the role), **Forgotten
passphrase** (a re-key: the same person, a new key), **Change my passphrase** (the same key, a new passphrase) and the
superuser's panel, **Superuser: enrollers and keys** (grant the role; Confirm or Repudiate a key). Paper counterpart:
a newcomer signs their own name on their first note, with no ceremony; a pen has no passphrase to forget. **Live
only**, like section 16.

**Budgets (design §7, the slice plan's §1.2).**

| Gesture | Start | Stop | Budget |
|---|---|---|---|
| Enrolment by an enroller | the newcomer says "I'm new" | the outcome line after the newcomer's **first own signature** | **≤ 2 min** |
| Re-key in person | *Forgotten passphrase* opened | the member's first signature with the new passphrase | **≤ 1 min** |
| Change my passphrase | *Change my passphrase* opened | "Your passphrase is changed." | **≤ 30 s** |
| Superuser confirm | the superuser starts typing their passphrase in the strip (it is emptied after every press) | the row leaves the worklist | **≤ 10 s per row** |

Steps: enrolment paper *N* = 0 → forced *M* = 2 (the enroller's form and signature; the newcomer's passphrase twice),
once per person per node → bundling target *K* = 2. Re-key paper 0 → *M* = 2 → *K* = 2. Change of passphrase paper 0
→ *M* = 1 → *K* = 1. Confirm: 1 press per row. **`M > N` is already filed** ([#919](https://github.com/cairn-ehr/cairn-ehr/issues/919)); record
the observed counts against it. **A figure outside its budget is the finding — file an issue; never adjust the
budget.**

**Setup.** Section 16's setup (the database, the owner connection, `cnode`, the logging check, a superuser `boss`
and the terminal passphrase). Enrol, from the window as section 16 Measurement 2, a clinician `dr-a` (role
*clinician*, approved by `boss`: confirmed at once). Then, at the console:

```bash
# The site's optional expiry (off by default). Measure with it OFF; Measurement 6 turns it on and off again.
cnode vault-set-unconfirmed-expiry --none
# Who is unconfirmed, and who approved what, straight from the registry (owner connection):
psql "$CONN" -At -c "SELECT m.staff_id, cairn_key_confirmation(m.signing_key_id) FROM vault_member m ORDER BY 1"
```

**Measurement 1 — grant the enroller role.** Open *Superuser: enrollers and keys*. Type `boss` and its passphrase
into the strip; press *Grant* beside `dr-a`.

- [ ] "dr-a now has the enroller role. They can enrol staff while their own key is confirmed." The row now says
      "has the enroller role" and offers *Withdraw*.
- [ ] `dr-a`'s own credential and *Grant* beside another member: "Only a superuser can do this. …" Nothing changes.
- [ ] The strip's passphrase is empty after every press, success or refusal.

**Measurement 2 — enrolment by an enroller (the ≤ 2 min budget).** A colleague plays the newcomer. Start when they
say "I'm new". `dr-a` opens *Enrol staff*, types the newcomer's details (`locum-b`, role *clinician*, a contact) and
their own credential into the strip; the newcomer types their own passphrase twice and presses *Enrol*; then the
newcomer signs one act on the section 2 chart with their own staff ID and passphrase. Stop at that act's outcome
line. Three runs (new staff IDs each time).

- [ ] The enroller never sees the newcomer's passphrase, and the newcomer never chooses anything but a passphrase.
- [ ] The newcomer's act is signed by their own key (`medication-list`, then the `psql` line above: `unconfirmed`).
- [ ] `dr-a` choosing role *admin* or *superuser*: "Only a superuser can enrol a superuser or an admin. …"
- [ ] The newcomer's key appears under *Keys waiting for a superuser*: "… new key approved by dr-a on …".

**Measurement 3 — re-key in person (the ≤ 1 min budget).** `locum-b` has "forgotten" their passphrase. `dr-a` opens
*Forgotten passphrase*, types `locum-b`, role *clinician*, and their own credential in the strip; `locum-b` types a
new passphrase twice and presses *Give a new key*; then signs one act with it. Stop at that act's outcome line.

- [ ] "locum-b has a new key. Their old passphrase no longer works. A superuser must confirm the new key." (an
      enroller approved). Re-key another member with `boss` in the strip: the line ends at "no longer works.", since
      a superuser's approval confirms the key at once.
- [ ] `locum-b` signing with the OLD passphrase: "This passphrase was replaced on {date}, approved by {dr-a's name}.
      If you didn't ask for that, tell a superuser." It counts as a wrong try: the fourth in a row says "Wait 1 s".
- [ ] The act signed with the new passphrase is `locum-b`'s, on the same actor
      (`psql "$CONN" -At -c "SELECT op, left(signing_key_id,8) FROM actor_event WHERE actor_id = (SELECT actor_id FROM vault_member WHERE staff_id='locum-b') ORDER BY seq"`:
      `enroll`, then `rotate-key`).
- [ ] *Role* offers no *superuser*. A superuser is re-keyed only at the console:
      `cnode vault-rekey-superuser --staff-id boss` (passphrase prompted twice), then `boss` signs in with the new one.
- [ ] Leave *Role* on *clinician* and re-key a *nurse*: "… Check that the role is the one they are enrolled with: a
      re-key never changes a role. …" Nothing changes.

**Measurement 4 — change my passphrase (the ≤ 30 s budget).** `dr-a` opens *Change my passphrase*, types staff ID,
current passphrase and a new one twice, and presses *Change passphrase*.

- [ ] "Your passphrase is changed." The next sign with the new passphrase works; the old one is "not right" (no
      "replaced": the key did not change).
- [ ] A wrong current passphrase: the ordinary refusal ("That passphrase is not right."), and it counts as a try.
- [ ] The same passphrase as the current one: "Your new passphrase is the same as your current one. Nothing was changed."

**Measurement 5 — confirm and repudiate (the ≤ 10 s per row budget).** Back in the superuser's panel as `boss`.
Time *Confirm* on `locum-b`'s row (five rows: enrol more newcomers first).

- [ ] "The key is confirmed." The row leaves the worklist; the `psql` line says `confirmed`.
- [ ] *Repudiate* on another newcomer: no question is asked (principle 3). "The key is repudiated. Give the member a
      new key from the re-key form." That member's next sign: "Your signing key is no longer enrolled here. Nothing
      was signed. Ask a superuser or an enroller to re-key you."
- [ ] **A stale row is refused.** Keep the panel open, re-key a newcomer from *Forgotten passphrase* with `dr-a` in
      the strip, then press *Repudiate* on that newcomer's OLD row without reopening the panel: "The node refused
      this, so nothing was recorded. The lists may have been out of date …". The lists redraw with the new key.
- [ ] **A confirmed key, and the cascade.** Leave one newcomer `dr-a` approved unconfirmed. Under *Who has the
      enroller role*, press *Repudiate key* on `dr-a`'s row (a confirmed key: it is not on the worklist). No question is
      asked. `dr-a`'s row then says "Their key is repudiated: give them a new key from the re-key form." with no
      *Repudiate key*, and every key `dr-a` approved that is still unconfirmed says "Doubtful: approved by a
      repudiated enroller." Time the press as a confirm row. `dr-a` trying to enrol now: "Your own signing key was
      repudiated, so you cannot approve anyone with it. …". Then re-key `dr-a` with `boss` in the strip (a
      superuser's re-key is confirmed at once, and the grant is kept), so Measurement 6 can enrol through `dr-a`.

**Measurement 6 — the optional expiry.** `cnode vault-set-unconfirmed-expiry --days 5`, then enrol a newcomer
through `dr-a` (unconfirmed): it stops signing in five days, inside the strip's seven-day warning. To see the stop itself on
this **throwaway** database only, back-date that key's registry rows past the limit as the tests do
(`vault_enrollers.rs`'s `age_key`: the owner disables `actor_event_no_update`, moves `recorded_at` back, and
re-enables it at once). Never on a real node: the registry is append-only. Then:

- [ ] Typing that member's staff ID in the strip (and leaving the field), within seven days of the stop: "Not yet
      confirmed by a superuser. {staff ID}'s key stops signing on {date}." Another staff ID clears it.
- [ ] The worklist row says "Stops signing on {date}." (or "Stopped signing on …" once past).
- [ ] Past the stop, a sign: "No superuser has confirmed your signing key, and this site stops unconfirmed keys
      signing after a set time. Nothing was signed. …" Confirm it in the panel; the next sign works.
- [ ] `cnode vault-set-unconfirmed-expiry --none` afterwards. Record that a restore comes back with no expiry
      (the policy is outside the backups).

**Measurement 7 — re-key after a restore. Run it LAST:** it empties the vault, so every member enrolled above has
to be re-keyed (or the database re-seeded) before any earlier measurement is repeated. On the throwaway database:

```bash
psql "$CONN" -c "TRUNCATE vault_attempt, vault_member CASCADE"   # the vault is outside backups; the registry stays
cnode vault-rekey-superuser --staff-id boss --display-name "Site Admin" --email it@example.invalid
```

- [ ] With `boss` in the strip, re-key `dr-a` from the window with name and contact blank: "This staff ID has no
      vault entry here, as after a restore. Enter the member's display name and a phone or an email, then try again.
      Nobody was re-keyed." `boss`'s release was not spent (no attempt counted).
- [ ] Fill in the name and an email and press again: "dr-a has a new key. Their old passphrase no longer works." (a
      superuser approved: confirmed at once). `dr-a` signs with the new passphrase, on the same actor
      (`actor_event` shows a new `rotate-key` row for it).

**Keyboard and screen reader, same pass as section 6** (VoiceOver on macOS, keyboard only):

- [ ] *Forgotten passphrase*, *Change my passphrase* and the superuser's panel open, fill and submit by keyboard
      alone; *Enrol staff*'s new *admin* role is reachable in its list.
- [ ] Every field of the two new forms is announced by its label, and each contact field reads its note ("…it must work without any
      hospital login."); *Grant*/*Withdraw*, *Repudiate key* and *Confirm*/*Repudiate* are
      announced with the row they belong to (record if VoiceOver reads the button without its row).
- [ ] Each outcome sentence (re-key, change, grant, confirm, repudiate, "replaced", the expiry warning) is read once,
      whole, from its status line, and none speaks a passphrase.
- [ ] A double press on *Confirm* sends one act (the row's buttons are disabled while it is in flight).
- [ ] Nothing takes focus on its own when the worklist redraws.

**The window's DB-gated tests (owed by a human — CI never runs them,
[#905](https://github.com/cairn-ehr/cairn-ehr/issues/905)).** With the dev cluster (`CAIRN_TEST_PG` as in
`docs/developers/getting-started.md`), from `cairn-gui/`:

```bash
CAIRN_TEST_PG="host=127.0.0.1 port=5532 user=$USER dbname=cairn_test" \
  cargo test -p cairn-gui-tauri -- --nocapture   # no pipe: `| tail` or `| grep` would hide cargo's exit code
```

- [ ] No `skipped:` line, and `an_enroller_rekeys_from_the_window`, `a_replaced_sign_names_the_approver`,
      `a_superuser_grants_and_an_enroller_then_rekeys`, `a_repudiated_key_leaves_the_worklist_for_good` and
      `a_superuser_rekeys_a_restored_member_from_the_window` pass.
- [ ] Never beside `scripts/run-db-gated-tests.sh` on the same database (a shared schema replay deadlocks).

**What could not be run here.** Every gesture, stopwatch figure and the VoiceOver pass need the window and a human.
The doors behind them were run against throwaway databases by the slice's DB-gated tests on 2026-11-01.
`TEMPLATE.md` carries a matching block (*Enrol in person, re-key, change a passphrase, confirm*).

## 18. Write, autosave and sign a progress note (GUI sign-in slice 3a, ADR-0087)

The slice's gestures: **New note**, typing (saved by itself at the first keystroke, then at most once a second while typing, R14), an optional
**When it happened**, and **Sign note** with the signer strip of section 16. Paper counterpart: the progress sheet in
the bedside folder: pick it up, write (heading the entry with the time it happened, if not now), sign. **Live only**:
a `--mock` window lists no notes and refuses every note write ("fixture mode…").

**Budgets (the slice plan's §1.2).**

| Gesture | Start | Stop | Budget |
|---|---|---|---|
| Open the editor | the press of *New note* | the caret blinking in *Progress note* | **≤ 1 s** |
| Sign a note, credential included | the first keystroke in the strip's Staff ID | "Signed — the note is on the chart." | **≤ 3 s** |
| Get a note back after a crash | the window reopened on the chart | the text back in the editor | **0 extra acts** |

Steps: writing and signing paper *N* = 3 (pick up the sheet, write, sign) → forced *M* = 3 (*New note*, type, sign) →
bundling target *K* = 3; a stated time is part of writing, as on paper. Reading a chart's notes 0 → 0 → 0. Getting a
note back after a crash: paper has no crash; *M* = 0. The write half of every sign is also timed by the node
(`gesture_kind = 'note-sign'`, section 5). **A figure outside its budget, or `M > N`, is the finding — file an issue;
never adjust the budget.**

**Setup.** Section 16's setup (the database with `cairn_pgx` 0.4.0, the owner connection, `cnode`, the logging check,
a superuser and the terminal passphrase) and `nurse-a` enrolled from the window. The section 2 chart. Launch as
section 3 with no `--key-hold-seconds`. The window keeps its terminal id in
`~/Library/Application Support/org.cairn-ehr.med-list/terminal-id` (macOS; the Tauri config directory elsewhere).
Read what the node holds with the owner connection:

```bash
notes()  { psql "$CONN" -c "SELECT left(narrative, 40) AS narrative, effective_basis, t_effective, recording_started, recording_ended FROM clinical_note ORDER BY t_effective"; }
drafts() { psql "$CONN" -c "SELECT left(p.narrative, 40) AS narrative, d.terminal_id, p.stated_time, p.last_keystroke FROM note_draft d JOIN note_draft_part p USING (draft_id) ORDER BY d.recording_started, p.seq"; }   # since 3b-1 (db/066) the text is in note_draft_part
```

- [ ] **The setup hook filled the terminal id.** After the first launch the file holds one UUID, and it is the same
      after a relaunch. A note typed straight after launch saves (no "this window is still starting" sentence).

**Measurement 1 — open the editor (≤ 1 s).** Press *New note*. Five runs (sign or reopen between runs: one draft is
written at a time).

- [ ] The editor shows with the caret in *Progress note*; *New note* is hidden and *Sign note* shown.
- [ ] Nothing is saved until something is typed (`drafts` lists no row for this press).

**Measurement 2 — write and sign (≤ 3 s).** Type `BP <90, [L] arm (R), see [x](http://example.org) \ ok`. Wait two
seconds, then type the strip's Staff ID and passphrase and press *Sign note*. Five runs, with and without a time in
*When it happened*.

- [ ] The listed note reads **exactly as typed**, with "signed by" nurse-a's name. `notes` shows the escaped narrative
      (`BP \<90, …`). Nothing was refused.
- [ ] With no time stated: the note reads "no stated time · writing began HH:MM", and `notes` shows
      `recording-started` with `t_effective` = `recording_started`. With a time: "HH:MM (stated)", basis `stated`.
- [ ] `recording_ended` is the last keystroke, not the press of Sign (the two-second wait shows as a gap).
- [ ] The press → outcome share of the gesture, and `ui_gesture_timing`'s `note-sign` row (section 5).
- [ ] **A partly typed time.** Type only part of *When it happened* (a date with no hour) and press *Sign note*: "the
      time it happened is only partly typed — finish it, or clear it if the note is about now". Nothing is signed and
      the text is kept. A saved stated time is not wiped by the partial one (`drafts` still shows it).
- [ ] **Read-only while signing.** In a second terminal, `psql "$CONN"` as the owner: `BEGIN; LOCK TABLE note_draft IN
      ACCESS EXCLUSIVE MODE;`. Press *Sign note*, then type in the editor and the time field: nothing appears and Sign
      is disabled. `ROLLBACK;` within 15 s: the sign completes with the text as it was at the press.
- [ ] **Stated seconds survive a reopen.** On this throwaway database only, give an open draft a time with seconds
      (`UPDATE note_draft_part SET stated_time = '2026-11-03T03:00:15+00' WHERE …`; `note_draft` before 3b-1), reopen the chart, and sign without
      touching the field: `notes` shows `t_effective` at `:15`.

**Measurement 3 — a crash loses nothing.** Type a note, wait two seconds, then kill the window hard
(`pkill -9 -f cairn-gui-tauri`, so no close-time save can run). Relaunch on the same chart.

- [ ] The draft is back in the editor, with its stated time, and the status line reads "unsigned draft · started … ·
      last typed …". Count every act before typing can continue (target 0). **Since 3b-1** a relaunch forgets who's here, and a
      draft reopens into the editor only once somebody has said who is writing: record the acts as they now are
      (expected: *Continue here* asks, one pick, then the editor) — section 19's Measurement 5 (#974).
- [ ] Kill it again within a second of a keystroke: record how much of the last typing was lost (the bound is the
      one-second save interval, R14).
- [ ] Stop the node's Postgres while typing: "not saved yet — your text is still here" shows and the text stays.
      *Sign note* says "Could not sign this note: …" and keeps the text. Start Postgres; the next keystroke saves.
- [ ] **Another terminal's draft.** Quit, move the terminal-id file aside, relaunch: the draft is listed, read-only,
      "… · being written at another terminal" (since 3b-1 with **Continue here**: do not press it here, section 19 times it). Put the file back and relaunch: it reopens in
      the editor (since 3b-1, after a who's-here pick).

**Keyboard and screen reader, same pass as section 6** (VoiceOver on macOS, keyboard only):

- [ ] *New note*, *Progress note*, *When it happened* and *Sign note* are reachable and usable by keyboard alone;
      Enter in the time field signs nothing.
- [ ] *Progress note* and "When it happened — leave empty if it is now" are announced by their labels.
- [ ] Each listed note is read with its text, then its grade and time ("signed by …", "authorship claimed, not verified
      here" or "writer not known"). On a doubted linked pair (section 13's rig), the withheld-notes sentence is read.
- [ ] The status line (`#note-status`) is read once per change. **Record whether the sign sentence is read twice**:
      it lands on `#note-status` and on the chart's `#outcome`, both live regions.
- [ ] No dialog appears at any point, and nothing takes focus on its own after a sign or a re-read.

**What could not be run here.** Every gesture, stopwatch figure and the VoiceOver pass need the real window. The
editor was walked headlessly in Chromium only (sign-in slice 3a, Task 6): WebKit's `readOnly`, `datetime-local`'s
partial input and seconds, Tauri's argument casing for `note_draft_save` / `note_sign`, and the setup hook that fills
the terminal id were never seen in the real window. The doors were run against throwaway databases by the slice's
DB-gated tests on 2026-11-03. `TEMPLATE.md` carries a matching block (*Write, autosave and sign a progress note*).

## 19. Who's here, the idle re-lock and a draft's parts (GUI sign-in slice 3b-1, ADR-0088)

The slice's gestures: **Who's here?** (the button in the bar, or Ctrl+Shift+H, Cmd+Shift+H on macOS) and its picker,
the **Return to …** offer, the **idle re-lock**, writing **below a colleague's part**, and **Continue here** on a draft
held at another terminal. Paper counterpart: the progress sheet in the bedside folder. Whoever picks up the pen writes
below what is there, nobody rubs out a colleague's ink, an open folder is put away when its reader walks off, and the
sheet is carried to wherever you sit down to finish it. **Live, except one check:** `--mock` lists two fixture staff
(Dr B, Nurse A), so the picker, the bar and the hot key can be walked there, but it holds no drafts and refuses every
note write.

**Budgets (the slice plan's §1.2).**

| Gesture | Start | Stop | Budget |
|---|---|---|---|
| Hot key to named, 50 staff on the node | the press of Ctrl/Cmd+Shift+H, a chart open | the bar reads "Writing as …" | **≤ 3 s** |
| Back at the chart after the re-lock | the first keystroke in the terminal passphrase | the chart on screen, header read | **≤ 10 s** |
| Continue here | the press of *Continue here* | the caret in *Progress note*, the draft's parts above | **≤ 1 s** |
| A switch mid-typing | the hot key pressed within a second of a keystroke | the switched-out writer's part, read back | **nothing lost** |

Steps: reading a chart 0 → 0 → 0 (no name asked) · writing and signing 3 → 3 (say who's here once per stint, type,
sign with only the passphrase, the staff ID prefilled) → 3 · adding to a colleague's draft 1 → 1 → 1 · finishing at
another terminal 1 (plus the walk) → 1 with the writer already named here, 2 (*Continue here* + the pick) with nobody
named (#974) · after a window restart 3a's 0 → about 2 (#974) · resuming after the re-lock 2 (plus the walk to the folder) → 3 (unlock,
pick yourself, *Return to …*) → 3, which the maintainer ruled is not an architecture defect (maintainer decision 3, recorded in ADR-0088 decision 5).
**A figure outside its budget, or another `M > N`, is the finding — file an issue; never adjust the budget.**

**Setup.** Section 18's setup, with **two** enrolled clinical members besides `boss`: `nurse-a` and `dr-b`, each
enrolled from the window (section 17) so each has a passphrase. Launch as section 3 (with `--key-hold-seconds 900`,
so a held key exists to be forgotten) plus `--idle-lock-minutes 1` for Measurement 3 only. For Measurement 1, add 48
placeholder members **on this throwaway database only** (they cannot sign: their sealed seed is a placeholder nobody
can release), so the directory holds 50 clinical staff:

```bash
psql "$CONN" -c "INSERT INTO vault_member (staff_id, display_name, role, actor_id, signing_key_id, sealed, contact_email, enrolled_by)
  SELECT 'staff-' || i, 'Staff ' || chr(64 + (i % 26) + 1) || i, CASE WHEN i % 3 = 0 THEN 'clinician' ELSE 'nurse' END,
         decode(md5('actor' || i), 'hex'), md5('k' || i) || md5('l' || i), '\x00', 's' || i || '@x', 'console'
    FROM generate_series(1, 48) i"
parts() { psql "$CONN" -c "SELECT d.draft_id, d.patient_id, d.terminal_id, p.seq, p.writer, left(p.narrative, 40) AS narrative, p.last_keystroke FROM note_draft d JOIN note_draft_part p USING (draft_id) ORDER BY d.recording_started, p.seq"; }
revs()  { psql "$CONN" -c "SELECT seq, writer, terminal_id, left(narrative, 30) AS narrative, saved_at FROM note_draft_revision ORDER BY rev"; }
signed() { psql "$CONN" -c "SELECT left(n.narrative, 30) AS narrative, n.refs, e.contributors FROM clinical_note n JOIN event_log e USING (event_id) ORDER BY n.recording_started"; }
```

**Measurement 1 — hot key to named (≤ 3 s).** A chart open, nobody typing. Press the hot key, type the first letters
of `dr-b`'s display name, and pick them (Enter picks the first match). Five runs with the person among this terminal's
recent people (listed first) and five with someone never picked here.

- [ ] The chart leaves the screen before the picker shows; the picker has no passphrase field; the search has focus.
- [ ] The bar reads "Writing as …" and the signer strip's Staff ID holds that person's staff ID; the passphrase field
      is empty. With `--key-hold-seconds 900` and a key held before the press, the strip no longer shows it held.
- [ ] The offer "Return to …" shows, focused, for a person who opened a chart from a list here before; it is never
      opened by itself. Esc dismisses it; Enter opens that chart. **Record what it shows after the name** (an age today,
      #972).
- [ ] *Don't know / just reading* names nobody ("Writer not known"); *Back* keeps the name as it was; Enter on an empty
      search picks nobody.

**Measurement 2 — a switch mid-typing loses nothing; a colleague writes below.** As `nurse-a`: *New note*, type
`obs stable, BP 120/80`, and press the hot key within a second of the last keystroke. Pick `dr-b`, reopen the chart.

- [ ] `parts` shows part 1 by `nurse-a` holding **every** character typed. Nothing `dr-b` types lands in it.
- [ ] As `dr-b`: Nurse A's part is drawn read-only above an empty editor, headed "?Nurse A". Type `plan: review 4h`:
      `parts` shows part 2 by `dr-b`. Switch back to `nurse-a` and type: part 3 by `nurse-a`, part 1 unchanged.
- [ ] `revs` shows one row per real change, each with its writer and terminal; an idle second adds none.
- [ ] Switch to `dr-b` and sign with the editor empty (only the passphrase typed). Three notes are listed, the `nurse-a` ones reading "written by
      ?Nurse A · signed by …". `signed` shows part 1 with `drafted` + `attested` contributors, part 2 with `authored`,
      and parts 2 and 3 each with an `addendum-to` ref to the part before. `parts` is empty, `revs` too.

**Measurement 3 — the idle re-lock (≤ 10 s back at the chart).** Relaunch with `--idle-lock-minutes 1`. Name
`nurse-a`, open a chart from a list, type a few words, then leave the terminal alone for a minute.

- [ ] The chart leaves the screen, the terminal lock shows with its passphrase field focused, the bar reads "Nobody has
      said who is at this terminal" and the strip's Staff ID is empty. `parts` holds the typed words.
- [ ] Time unlock → pick `nurse-a` → *Return to …* → the chart on screen; count the acts (budget 3).
- [ ] Typing steadily in any field, the terminal passphrase field included, never re-locks; leaving the window open
      with no touch while the lock-state poll runs does re-lock (a poll is not activity).
- [ ] Press *Sign note* and hold the sign (section 18's `LOCK TABLE note_draft` trick) past the minute: the re-lock
      waits until the sign settles. Record what the clinician is told (by ruling nothing on the lock screen, #970).

**Measurement 4 — Continue here (≤ 1 s), and the move.** Type a draft as `nurse-a`, quit, move the terminal-id file
aside (a new terminal id), relaunch and name `nurse-a`.

- [ ] The draft is listed "… · being written at another terminal", with *Continue here*. Time its press to the caret.
      The draft is now this terminal's (`parts`' `terminal_id`).
- [ ] With the editor open on it, move it away from the owner connection:
      `psql "$CONN" -c "SELECT cairn_note_draft_take('<draft id>', '<patient id>', 'elsewhere-test')"`. Type one more
      word: the status line says the note is now being continued at another terminal, and the editor keeps only the
      new word as a new draft of its own (`parts` shows two drafts). Repeat with no new typing (the editor closes, the
      sentence says everything was saved into it) and after editing the saved text (the whole editor is kept, and
      said so).
- [ ] **Signed in another window** (final review I1, #945). With the editor open on a draft here, press *Continue
      here* on it at a second terminal, add a line there and sign it. Type one more word here: the status line says
      "This note was signed in another window. What you typed since the last save is kept here as a new draft.", and
      the editor keeps only the new word, as a new draft of its own. Repeat pressing *Sign note* here instead of
      typing: the refusal ("already signed") is followed by the same sentence, and the editor is never left on the
      signed draft.

**Measurement 5 — resuming with nobody named (#974; ruled not an `M > N` defect, ADR-0089 decision 13).** Two
cases the plan's step counts left out. Count the acts and time each one, five runs: the counts are still recorded.

- [ ] **Continue here with nobody named.** As in Measurement 4, but relaunch and do NOT pick anyone first: the bar
      reads "Nobody has said who is at this terminal". Press *Continue here*: the picker opens. Pick `nurse-a`. Count the
      acts from the press to the caret in *Progress note* (paper 1; expected 2), and time them.
- [ ] **Reopening a draft after a window restart.** Type a draft as `nurse-a`, wait two seconds, kill the window hard
      (`pkill -9 -f cairn-gui-tauri`) and relaunch on the same chart. The draft is listed, not in the editor. Count every
      act before typing can continue (3a's target 0; expected about 2: *Continue here* + the pick), and time them. Also
      record whether the terminal lock asked for its passphrase first (it is a separate gatekeeping act).

**Keyboard and screen reader, same pass as section 6** (VoiceOver on macOS, keyboard only):

- [ ] The hot key, the picker (type, Enter, Tab to a name, *Don't know*, *Back*, Esc) and the offer work by keyboard
      alone; after the picker closes focus is on *Who's here?* (a chart open) or the search (no chart), never lost.
- [ ] The bar is read once when the name changes, and not at each lock-state poll.
- [ ] A colleague's part is read as "unverified Nurse A", then its text; a part with no writer as "writer not known".
- [ ] A signed note Nurse A typed and Dr B signed is read "written by unverified Nurse A · signed by Dr B" (#973),
      never with a bare "question mark"; one signed under "Don't know" reads "writer not known · signed by Dr B".
- [ ] No dialog appears at any point.

**Not a fault: a part 2 refused at a peer** ([#948](https://github.com/cairn-ehr/cairn-ehr/issues/948), until it is
decided). A two-writer note signs part 2 as an `addendum-to` part 1, and set-union sync does not promise order. A peer
that holds the body's key and receives part 2 before part 1 refuses it, and the pull reports it as failed. Do not
chase it: the refused part is penned and re-offered, and the failure heals once part 1 has landed.

**What could not be run here.** Every stopwatch figure and the VoiceOver pass need the real window. The picker, the
bar, the hot key, the switch order, the parts, *Continue here*, the moved fork and the re-lock were walked headlessly
in Chromium only, against a stubbed `invoke` (sign-in slice 3b-1, Task 6; the signed-in-another-window fork, the
`mayBeSigned` mark and the spoken byline in the final fix wave); WebKit, Tauri's argument casing for
`whos_here_pick`, `note_draft_take` and `relock`, and real timers over a real minute were not. The doors were run
against throwaway databases by the slice's DB-gated tests. `TEMPLATE.md` carries a matching block (*Who's here, the
idle re-lock and a draft's parts*).

## 20. A draft left untouched goes in unsigned (GUI sign-in slice 3b-2a, ADR-0089)

The slice has no new gesture: the point is that **nobody acts**. A draft nobody has typed into for the site's
quarantine goes into the record by itself, node-signed, and reads "went in unsigned". Paper counterpart: an entry left
unsigned on the progress sheet. It stays in the folder, readable by anyone, plainly not signed, and nobody acts to put
it there. **Live only**: a `--mock` window holds no drafts and commits nothing.

**Budgets (the slice plan's §1.2).**

| What | Start | Stop | Budget |
|---|---|---|---|
| The tick commits an overdue draft | the draft's due time (`due` below) | its note's row in `clinical_note` | **≤ 60 s** |
| A chart open that commits first | the press that opens the chart | the header read, the note listed | **≤ the single-chart open + 200 ms** |
| Reading a note that went in unsigned | — | — | **0 acts** |

Steps: walking away from a draft paper *N* = 0 → forced *M* = 0 → target *K* = 0; reading a note that went in
unsigned 0 → 0 → 0; setting the quarantine is not a clinical act (the owner, at the console, once). **A delay, not
an `M > N` case:** paper's unsigned entry is in the folder at once, Cairn's only after the quarantine. **A figure
outside its budget, or an `M > N`, is the finding — file an issue; never adjust the budget.**

**Setup.** Section 19's setup (`nurse-a` and `dr-b` enrolled from the window) and **two** charts, A and B (the
section 2 chart and one more). Launch as section 3, which passes `--key "$NODE_KEY"`. Set the quarantine to two minutes at the console, and put it back at the end:

```bash
cnode note-set-quarantine --minutes 2     # prints: a draft left untouched for 2 minutes goes into the record unsigned
cnode note-set-quarantine --default       # at the end: 180 minutes (the default)
due()    { psql "$CONN" -c "SELECT d.draft_id, q.due, q.overdue, q.stuck, clock_timestamp() AS now FROM note_draft d, cairn_note_drafts_due(d.patient_id) q WHERE q.draft_id = d.draft_id"; }
went()   { psql "$CONN" -c "SELECT left(n.narrative, 30) AS narrative, e.contributors, e.signer_key_id, s.by_timer FROM clinical_note n JOIN event_log e USING (event_id) LEFT JOIN note_draft_spent s ON s.event_id = n.event_id ORDER BY n.recording_started"; }
```

`parts`, `revs` and `signed` are section 19's helpers.

**Measurement 1 — the tick (≤ 60 s past due), with the window on another chart.** As `nurse-a` on chart A: *New
note*, type `obs stable`. Switch to `dr-b`, reopen chart A and type `plan: review 4h` below (a two-writer draft).
Note the `due` time and check the first box below **while still on chart A, before it is due**. Then open chart B and
leave the terminal alone (with `--idle-lock-minutes` above 3, so the re-lock does not close chart B first), and watch
the log in a second terminal with `\watch 1` on `went`'s query. Five runs.

- [ ] On chart A, before it is due, the draft reads "… · goes into the record unsigned at HH:MM if nobody types in
      it", HH:MM matching `due`.
- [ ] Record the seconds from `due` to the rows appearing (budget ≤ 60 s). `went` shows **two** notes:
      `[{nurse-a's key, drafted}, {node, recorded}]` and `[{dr-b's key, drafted}, {node, recorded}]`, both signed by
      the node key, the second an `addendum-to` the first (`signed`). `parts` and
      `revs` are empty. `by_timer` is true on the FIRST note's row only (the spend names the first event;
      the second's reads empty).
- [ ] Open chart A: two notes, read "written by ?Nurse A · went in unsigned" and "written by ?Dr B · went in
      unsigned". No "signed by" anywhere on them.

**Measurement 2 — a chart open that commits first (≤ the single-chart open + 200 ms).** Quit the window so no tick
runs. Leave a draft on chart A until `due` says `overdue` is true. Relaunch on chart B, then open chart A from a list
before the first tick (it waits a minute after launch). Five runs, and five opens of chart A with no draft for the
baseline.

- [ ] Chart A shows the note, "went in unsigned", and no draft. Record the open times against the baseline.

**Measurement 3 — typing into a just-committed draft.** With the editor open on chart A's draft, stop typing past
`due` and wait for the tick (`went` shows the note). Then type `; later: BP 90/60`.

- [ ] The status line reads "This note went into the record unsigned after it was left untouched. What you typed
      since the last save is kept here as a new draft.", and the editor holds only the new text, as a new draft
      (`parts` shows a new draft id). Repeat, but after the commit first delete a word of the text that was saved, then
      type: the status line reads the "Everything on this screen is kept here as a new draft; what was saved before is
      also in that note." sentence, and the whole editor is kept.
- [ ] Repeat pressing *Sign note* instead of typing (with the passphrase typed): "Could not sign this note: this note
      had already gone into the record unsigned — nothing was signed", then the new-draft sentence above (or the
      editor closes, when nothing was unsaved). Nothing is signed (`went`).

**Measurement 4 — a stuck draft (on this throwaway database only).** Leave a draft by `nurse-a`, then remove her
member row as the owner: `psql "$CONN" -c "DELETE FROM vault_attempt WHERE staff_id = 'nurse-a'; DELETE FROM
vault_member WHERE staff_id = 'nurse-a'"`. Check the first box **at once, before `due`**; then wait past `due` and
two ticks.

- [ ] Before `due`: `due` already shows `stuck` true, and the draft reads "… · cannot go into the record by itself —
      a writer of it is no longer enrolled on this node; it can go in only once they are enrolled again" (no time).
- [ ] After `due` and two ticks: the draft is still there (`parts`), nothing went in (`went`), and it reads "… ·
      overdue, but cannot go into the record by itself — a writer of it is no longer enrolled on this node; it goes in
      once they are enrolled again".
- [ ] The window's stderr says the draft once, not at every tick.

**Keyboard and screen reader, same pass as section 6** (VoiceOver on macOS, keyboard only):

- [ ] A note that went in unsigned is read "written by unverified Nurse A · went in unsigned", never with a bare
      "question mark", and never "signed".
- [ ] A draft's due words are read with the draft, and the overdue and stuck sentences in full.
- [ ] The committed sentence on `#note-status` is read once.
- [ ] No dialog appears at any point.

**What could not be run here.** Every stopwatch figure and the VoiceOver pass need the real window. The commit door,
the race, the stuck case and the grade were run against throwaway databases by the slice's DB-gated tests, and the
pre-read commit by `cairn-gui-tauri`'s DB-gated tests (CI never runs those, #905). The committed fork was walked
headlessly in Chromium only, against a stubbed `invoke` (sign-in slice 3b-2a, Task 4); WebKit, a hidden or
minimised window's tick over a real minute, and Tauri's runtime spawning the tick were not seen. `TEMPLATE.md`
carries a matching block (*A draft left untouched goes in unsigned*).

## 21. Throw away your own draft (GUI sign-in slice 3b-2b, ADR-0090)

The slice's gesture: **Discard draft**, beside *Sign note*, with the signer strip of section 16. The draft leaves the
chart's drafts; the log keeps a signed record of what it said, and the chart lists it as one closed line, "Draft
discarded by Nurse A · HH:MM", its text behind a click. Paper counterpart: tearing up your own page before it goes in
the folder. You cannot tear a colleague's writing out of it. **Live only**: a `--mock` window holds no drafts and
refuses every note write ("fixture mode…").

**Budgets (the slice plan's §1.2).**

| Gesture | Start | Stop | Budget |
|---|---|---|---|
| Discard a draft, credential included | the first keystroke in the strip's Staff ID | "Discarded. The draft is gone from the chart; the record keeps what it said." | **≤ 3 s** (Sign's, section 16) |
| See that a draft was discarded | — | — | **0 acts** (the marker is in the list) |
| Read what it said | the marker | its text | **1 press** |

Steps: discarding your own draft paper *N* = 1 (tear it up) → forced *M* = 1 (*Discard draft* + the strip, counted as
Sign is) → target *K* = 1; a draft of unknown writing 1 → 1 → 1; a draft holding a colleague's part: paper permits no
tear-up → refused; it goes in by the timer, or stays stuck (section 20) → no gesture. The write half is also timed by the node
(`gesture_kind = 'note-discard'`, section 5). **A figure outside its budget, or an `M > N`, is the finding — file an
issue; never adjust the budget.**

**Setup.** Section 19's setup (`nurse-a` and `dr-b` enrolled from the window) and the section 2 chart. Launch as
section 3. `parts` is section 19's helper; add:

```bash
discards() { psql "$CONN" -c "SELECT e.event_id, e.signer_key_id, e.contributors, e.t_effective, d.parts FROM clinical_note_discard d JOIN event_log e USING (event_id) ORDER BY e.t_effective"; }
spends()   { psql "$CONN" -c "SELECT draft_id, event_id, spent_at, by_timer, discarded_by FROM note_draft_spent ORDER BY spent_at"; }
```

**Measurement 1 — discard your own draft (≤ 3 s).** As `nurse-a`: *New note*, type `obs stable, BP 120/80`, wait two
seconds, then type the strip's Staff ID and passphrase and press *Discard draft*. Five runs.

- [ ] Record the time from the first keystroke in the Staff ID to "Discarded. The draft is gone from the chart; the
      record keeps what it said." (budget ≤ 3 s), and `ui_gesture_timing`'s `note-discard` row (section 5).
- [ ] No dialog. The editor closes; the draft is gone from the drafts (`parts`); no note was added.
- [ ] `discards` shows one event, signed by nurse-a's key, contributors `[{nurse-a's key, authored}, {node,
      recorded}]`, one part with `writer` `discarder` and the text as typed (escaped, as a note's is). `spends` shows
      its spend with `discarded_by` = nurse-a's key and `spent_at` = the event's `t_effective`.
- [ ] Repeat with an empty editor (*New note*, type a letter, delete it, *Discard draft*): "Discarded. The draft was
      empty, so nothing was kept." No event, no spend, no marker.
- [ ] With the strip empty and no key held, press *Discard draft*: "Not discarded: Enter your staff ID and passphrase
      to discard this draft." A key refusal under Discard never says "sign" (PR #984 review).

**Measurement 2 — a colleague's draft.** As `nurse-a` type `obs stable`; switch to `dr-b` and reopen the chart (Nurse
A's part is drawn read-only above dr-b's editor, as in section 19, Measurement 2), then press *Discard draft* with
dr-b's credential.

- [ ] The status line reads "Not discarded: someone else wrote part of this draft, and only its writer may throw
      their part away — nothing was thrown away; your text is still here." The draft is
      still there with nurse-a's part (`parts`), the editor still holds any text dr-b typed (it is saved again), and
      `discards` and `spends` are unchanged.
- [ ] Reopen the chart: the draft row's words say when it goes into the record unsigned (section 20's words), and the
      window offers no other step. (The editor itself shows no such line: #988.)

**Measurement 3 — a stuck draft (on this throwaway database only).** Leave a draft by `nurse-a`, then remove her
member row as the owner (section 20, Measurement 4's two `DELETE`s). As `dr-b`, reopen the chart and press *Discard
draft*.

- [ ] The same "Not discarded: someone else wrote part of this draft …" sentence; the draft is kept. Reopen the
      chart: the draft row's words say "stuck" (section 20's stuck sentence), never a time.

**Measurement 4 — a discard during the debounce.** As `nurse-a`, type `obs stable` and, within half a second of the
last keystroke, type `; recheck 1h` and press *Discard draft* at once (the strip filled beforehand).

- [ ] The marker's text (and `discards`' part) ends `; recheck 1h`: the audit copy holds what was on screen, not the
      last autosave.

**Measurement 5 — a who's-here switch during a discard.** Press *Discard draft* and, before the outcome shows, press
the hot key (Ctrl+Shift+H, Cmd+Shift+H on macOS).

- [ ] The picker opens only after the discard settles, and says "Your draft was discarded." above its heading. With
      the strip's passphrase wrong, it says "Your draft was NOT discarded — open the chart to see where it stands."
      It never says "signed".

**Keyboard and screen reader, same pass as section 6** (VoiceOver on macOS, keyboard only):

- [ ] *Discard draft* is reached by Tab after *Sign note* and pressed with Space or Enter; both are down while it runs.
- [ ] The marker is a closed disclosure ("Discarded drafts" list): VoiceOver reads its summary line and that it is
      collapsed; Space or Enter opens it, and each part's heading and text are read; the text is read as typed (markup
      characters as characters).
- [ ] The outcome on `#note-status` is read once; a refusal's sentence in full.
- [ ] No dialog appears at any point.

**Chromium and WebKit.** The window is WebKit (Tauri on macOS). Compare each step above with the headless walk
(below), and note any difference: the disclosure's keyboard behaviour, focus after the editor closes, and the
read-only editor during the discard.

**What could not be run here.** Every stopwatch figure and the VoiceOver pass need the real window. The door, the
rule, the race with a Sign, the commit-time refusal and the marker read were run against throwaway databases by the
slice's DB-gated tests, and the window's discard by `cairn-gui-tauri`'s DB-gated tests (CI never runs those, #905).
The button, the marker, a refusal, the debounce and a switch during a discard were walked headlessly in Chromium and
WebKit (Playwright, against a stubbed `invoke`; sign-in slice 3b-2b, Task 6); the real Tauri window, its argument
casing and VoiceOver were not seen. `TEMPLATE.md` carries a matching block (*Throw away your own draft*).

## 22. Add to a note, and read a note with its parts (GUI sign-in slice 3c-1, ADR-0091)

The slice's gestures: **Add to this note** on a signed note of the chart on screen, then type and *Sign note* with
section 16's strip; and reading a **compound note**: the first writer's heading and a state line ABOVE one text, each
later part led by a seam line ("Added: {its writer} · {its time}"), and a closed **"{n} parts"** disclosure holding
each part with its own heading. Paper counterpart: writing below an entry
on the progress sheet and signing; the page reads as one entry, and the handwriting shows who wrote which lines.
**Live only**: a `--mock` window holds no notes and refuses every note write ("fixture mode…").

**Budgets (the slice plan's §1.2).**

| Gesture | Start | Stop | Budget |
|---|---|---|---|
| Add to a note and sign it, credential included | the first keystroke in the strip's Staff ID | the addendum inside the note, the editor closed | **≤ 3 s** (Sign's, section 16) |
| Open a compound's parts | the press on "{n} parts" | every part's heading and text on screen | **≤ 1 s** |
| See that a note includes an unsigned part | — | — | **0 acts** (the state line) |

Steps: adding to a note and signing it paper *N* = 3 (find the entry, write below it, sign) → forced *M* = 3 (*Add to
this note*, type, Sign + the strip) → target *K* = 3; reading a note with additions 0 → 0 → 0; seeing who wrote which
part, and when, 0 → 0 → 0 (each later part's seam line); adding to a linked chart's note 3 → 3 →
**4** (open that chart first; a UI limit, #993). The write half is timed by the node (`gesture_kind = 'note-sign'`,
section 5). **A figure outside its budget, or an `M > N`, is the finding — file an issue; never adjust the budget.**

**Setup.** Section 19's setup (`nurse-a` and `dr-b` enrolled from the window) and the section 2 chart, with one
signed note on it by `nurse-a` (*New note*, `obs stable, BP 120/80`, a stated time, *Sign note*). Launch as section 3.
`signed` is section 19's helper (it shows each note's `refs`).

**Measurement 1 — add to a note and sign it (≤ 3 s).** As `dr-b`: press *Add to this note* on Nurse A's note. Five
runs: the first on Setup's note; before each later run, sign a new note as `nurse-a` (Setup's steps) and add to that
one.

- [ ] The editor opens with "Adding to the note of {its time}" above the text box. With nobody named at who's here,
      the picker asks first and the editor opens only once it is answered.
- [ ] Type `reviewed, plan unchanged`, then the strip's Staff ID and passphrase, then *Sign note*. Record the time
      from the first keystroke in the Staff ID until the addendum shows inside Nurse A's note (budget ≤ 3 s), and
      `ui_gesture_timing`'s `note-sign` row.
- [ ] No dialog. The chart shows ONE note: Nurse A's heading ("signed by Nurse A · {its time}") and "every part
      signed" ABOVE its text, never under the addendum's words; the addendum follows Nurse A's text after its seam
      line, "Added: signed by Dr B · {the addendum's time}"; then a closed "2 parts". `signed` shows the addendum's
      `refs` as `[{"rel": "addendum-to", "event": <Nurse A's note>}]`. The note keeps its place in the list (it is
      placed by Nurse A's time, not the addendum's).
- [ ] Press *Sign note* within half a second of the last keystroke of a new addendum: it still lands inside the note,
      never as a new note.
- [ ] Close the window mid-addendum (text typed, not signed) and relaunch: the draft reopens with its "Adding to …"
      line, and signs inside the note.
- [ ] Two terminals (or two windows on two terminal ids), `nurse-a` and `dr-b`, each add to the same note and sign
      within a few seconds of each other: both addenda show inside the one note, in the order they were begun.

**Measurement 2 — read a note with its parts (≤ 1 s).** Let a draft added to a note go in by the timer (section 20,
quarantine set low), so one part went in unsigned.

- [ ] The note's state line reads "includes an unsigned part" with no press, and the timer's part's seam line names
      its writer with the `?` and "went in unsigned" ("Added: written by ?{its writer} · went in unsigned · …").
- [ ] Press "3 parts" (or however many): record the time until every part's heading and text are on screen (budget
      ≤ 1 s). Each part reads with its own writer and how it went in ("went in unsigned" for the timer's part).

**Measurement 3 — the orphan heading (on this throwaway database only).** First, as `dr-b`, press *Add to this
note* on Nurse A's note (Setup's), type a line and leave the draft unsigned, with section 20's quarantine back above a
few minutes so the timer does not commit it. Then, as the owner, hide Nurse A's note from the read:
`psql "$CONN" -c "DELETE FROM clinical_note WHERE event_id = '<Nurse A's note>'"`. Reopen the chart.

- [ ] The addendum shows as a note of its own, its heading led by "Adds to a note not shown here · ", never as an
      ordinary note. The unsigned draft started before the `DELETE` reads "Adding to a note not shown here".
- [ ] Put the row back (`psql "$CONN" -c "SELECT cairn_reproject('clinical.note.')"`) and reopen the chart: one note
      again.

**Keyboard and screen reader, same pass as section 6** (VoiceOver on macOS, keyboard only):

- [ ] *Add to this note* is reached by Tab and pressed with Space or Enter; VoiceOver reads it as "Add to this note:
      {the note's time}", so a list of notes is not a list of identical buttons. A linked chart's note has none.
- [ ] The "{n} parts" disclosure is read as collapsed; Space or Enter opens it, and each part's heading and text are
      read; a claimed name reads "unverified …" (section 19).
- [ ] The "Adding to …" line is read with the editor; the heading and the state line are read BEFORE the note's text.
- [ ] Each seam line is read before its part's text, naming that part's writer and time; a claimed name reads
      "Added: written by unverified …", never a bare `?`. Tab and the arrow keys reach no seam as a control (it is
      text).
- [ ] No dialog appears at any point.

**Chromium and WebKit.** The window is WebKit (Tauri on macOS). Compare each step above with the headless walk
(below), and note any difference: the disclosure's keyboard behaviour, focus when Add opens the editor, and whether a
button drawn before the editor opened is still on screen (it does nothing, #994).

**What could not be run here.** Every stopwatch figure and the VoiceOver pass need the real window. The doors, the
floor rule, the header-sourced target, the timer's addendum, two addenda signed at once and #948's penned refusal were
run against throwaway databases by the slice's DB-gated tests, and the window's commands by `cairn-gui-tauri`'s
DB-gated tests (CI never runs those, #905). The compound, "Add to this note", the "Adding to …" line, a reopened
addendum draft and the target on every save and Sign were walked headlessly in Chromium and WebKit (Playwright,
against a stubbed `invoke`; sign-in slice 3c-1, Task 6), and the heading-above-the-text order and the seam lines
again in the final fix wave; the real Tauri window, its argument casing (`addendumTo`)
and VoiceOver were not seen. `TEMPLATE.md` carries a matching block (*Add to a note*).

## 23. Sign off a note, and sign your own earlier part later (GUI sign-in slice 3c-2, ADR-0092)

The slice's gestures: **Sign off this note** on a note that still needs a sign-off, with section 16's strip; and an
addendum's **Sign note** that also signs the writer's own earlier unsigned part, which the editor names on its "Your
signature also covers …" line before the press. The chart's new words are: "(student)" and "— not co-signed" on a
student's signed part; "(signer unconfirmed)"; a vouch's " · signed later by …", " · signed off by …" or " ·
co-signed by …"; and the state line "signed off by …". Paper counterpart: a supervisor countersigning beneath a
student's entry, and a nurse who comes back, writes a line and signs, covering her own earlier unsigned line.
**Live only**, like section 16.

**Budgets (the slice plan's §1.2).**

| Gesture | Start | Stop | Budget |
|---|---|---|---|
| Sign off a note, credential included | the first keystroke in the strip's Staff ID | "Signed off — …" on screen and the byline re-read | **≤ 3 s** (Sign's, section 16) |
| Finish your own unsigned note | the press of *Add to this note* | the addendum inside the note, the earlier part reading "signed later by …" | Sign's **≤ 3 s** from the strip's first keystroke |
| See that a student's part is not yet countersigned | — | — | **0 acts** (the byline) |

Steps: countersigning a note paper *N* = 2 (read, sign) → forced *M* = 2 (*Sign off this note*, the strip's
passphrase) → target *K* = 2; finishing one's own unsigned note 3 → 3 (*Add to this note*, type, Sign) → 3; seeing
a student's part not yet countersigned 0 → 0 → 0. The write half is timed by the node (`gesture_kind =
'note-sign-off'` for the sign-off, `'note-sign'` for the addendum, section 5). **A figure outside its budget, or an
`M > N`, is the finding — file an issue; never adjust the budget.**

**Setup.** Section 19's setup (`nurse-a` and `dr-b` enrolled from the window) and the section 2 chart. Enrol from
*Enrol staff*, approved by `boss`, two students: `sam` ("Sam") and `sue` ("Sue"), each with role **student**. The
role list offers *student* after *nurse*, in *Enrol staff* and in the re-key form. For Measurement 3 you also need a
member whose key is still **unconfirmed**: one an enroller approved and no superuser has confirmed (section 17's
`locum-b`, or enrol one the same way). Check with section 17's `cairn_key_confirmation` query. Launch as section 3.
`signed` is section 19's helper. Add:

```bash
vouches() { psql "$CONN" -c "SELECT a.target, a.act, a.role, m.staff_id AS by, a.patient_id FROM clinical_note_attestation a LEFT JOIN vault_member m ON m.signing_key_id = a.signer_key_id ORDER BY a.hlc_wall, a.hlc_counter"; }
```

**Measurement 1 — sign off a student's note (≤ 3 s).** As `sam`: *New note*, `obs stable, BP 120/80`, a stated time,
*Sign note*. Five runs, each on a new note of Sam's.

- [ ] Sam's note reads "signed by Sam (student) — not co-signed (no supervisor recorded) · {its time}" (the suffix since
      slice 3c-3a: nobody supervises Sam, section 25) and offers *Sign off this note*. A note `dr-b` signed alone offers
      none.
- [ ] As `dr-b` (who's here and the strip): record the time from the first keystroke in the strip's Staff ID, through
      the press of *Sign off this note*, until "Signed off — every part of this note now carries your signature."
      shows and the note is re-read (budget ≤ 3 s), and `ui_gesture_timing`'s `note-sign-off` row.
- [ ] No dialog. The byline now reads "signed by Sam (student) · co-signed by Dr B", with no "— not co-signed", and
      the button is gone. `vouches` shows one row: Sam's note, `sign-off`, `co-signed`, by `dr-b`, on Sam's chart.
- [ ] Sue presses *Sign off this note* on another of Sam's notes: it reads "… · co-signed by Sue (student)" and STILL
      "— not co-signed", and the button stays. With no supervisor recorded, only a non-student's co-sign clears the mark
      (ADR-0094 decision 13).
- [ ] Sam presses *Sign off this note* on his own note: "You have already signed every part of this note. Nothing
      more was signed." Nothing is written.
- [ ] Two terminals, both showing one of Sam's notes. On one, `nurse-a` adds to it and signs. On the other, `dr-b`,
      without reopening the chart, presses *Sign off this note*: "This note changed since it was shown — read the
      chart again, then sign it off. Nothing was signed." The pane re-reads, and *Sign off this note* is still offered
      on the note, now two parts.
- [ ] While a sign-off press is in flight, *New note*, *Add to this note* and *Continue here* do nothing, and the
      button is down until the press settles.

**Measurement 2 — finish your own unsigned note (≤ 3 s), and the covers line.** As `nurse-a`, write a note and let
it go in by the timer (section 20, quarantine set low; put it back above a few minutes afterwards). It reads
"written by ?Nurse A · went in unsigned · …".

- [ ] With who's here naming `nurse-a`, press *Add to this note*. Below "Adding to the note …" the editor reads "Your
      signature also covers your earlier unsigned part of this note: begun {its time}" (or its stated time with
      "(stated)", if one was stated). It is text, not a question, and nothing is asked before the press.
- [ ] Type `reviewed, all done`, fill the strip as `nurse-a`, press *Sign note*: "Signed — the note is on the chart.
      Your earlier unsigned part now reads as signed." Record the time (budget ≤ 3 s). The first part's byline now
      ends " · signed later by Nurse A", and the state line reads "every part signed", not "includes an unsigned part".
- [ ] Close the window mid-addendum (text typed, not signed) and relaunch. The draft reopens with its "Adding to …"
      line AND its covers line, and its Sign covers the part.
- [ ] **Review Focus 1.** Repeat with a fresh unsigned part of `nurse-a`'s. Who's here names `nurse-a`, but fill the
      strip as `dr-b`. The covers line names Nurse A's part. Sign: "Signed — the note is on the chart. 1 listed part(s)
      were not covered: not yours by key, or already signed." Nurse A's part still reads "went in unsigned" with no
      "signed later by", and `vouches` has no new row.
- [ ] As `dr-b`, sign off that note: the state line reads "signed off by Dr B", and the timer's part reads "… · signed
      off by Dr B".

**Measurement 3 — the unconfirmed mark.** As the unconfirmed member, sign a note. Its byline reads "signed by {them}
(signer unconfirmed)". As `boss`, *Confirm* their key (section 17), then reopen the chart: the mark is gone.

**Keyboard and screen reader, same pass as section 6** (VoiceOver on macOS, keyboard only):

- [ ] *Sign off this note* is reached by Tab and pressed with Space or Enter. VoiceOver reads it as "Sign off this
      note: {the note's time}", so a list of notes is not a list of identical buttons.
- [ ] The byline's "(student) — not co-signed", "(signer unconfirmed)" and each " · co-signed by …" / " · signed
      later by …" are read with the note's heading. A claimed name still reads "unverified …" (section 19).
- [ ] The covers line is read with the editor, after the "Adding to …" line and before the text box.
- [ ] The outcome lines ("Signed off — …", "This note changed …", "You have already signed …") are read when they
      appear.
- [ ] No dialog appears at any point.

**Chromium and WebKit.** The window is WebKit (Tauri on macOS). Compare each step with the headless walk below and
note any difference: focus after a sign-off re-reads the pane; whether a button drawn before the editor opened is
still on screen (it does nothing, #994, now *Sign off this note* too); a forked draft's covers line (#1001).

**What could not be run here.** Every stopwatch figure and the VoiceOver pass need the real window. Several things
were run against throwaway databases by the slice's DB-gated tests: the floor at both doors, the projection, the
read's two checks, the sign-off as shown, two sign-offs at once, an unplaceable signer's part, a shredded part, and
the addendum's vouch with key and claim differing. The window's commands were run by `cairn-gui-tauri`'s DB-gated
tests (CI never runs those, #905). The button, the gate, the in-flight guards, the covers line and `alsoVouches` on
Sign were walked headlessly in Chromium only (Playwright, against a stubbed `invoke`; sign-in slice 3c-2, Task 6).
WebKit, the real Tauri window, its argument casing (`shownParts`, `alsoVouches`) and VoiceOver were not seen.
`TEMPLATE.md` carries a matching block (*Sign off a note*).

## 24. A repudiated signature counts as unsigned, until its writer re-signs (GUI sign-in slice 3c-3b, ADR-0093)

The slice's gesture: the real person's **Sign off this note** on their own part, signed under a key a superuser has
since repudiated, with section 16's strip. The chart's new words are: " (key had been repudiated)" after a signer's
name; " — counts as unsigned" on such a part until it is re-signed; " · re-signed by …" on the vouch that re-signs
it; and the state line "includes an unsigned part" on a note of two or more parts that holds one. Paper counterpart:
an entry signed by an impostor is annotated, never torn out, and the real clinician reads it and signs it again.
**Live only**, like section 16.

**Budgets (the slice plan's §1.2).**

| Gesture | Start | Stop | Budget |
|---|---|---|---|
| Re-sign one's own repudiated part, credential included | the first keystroke in the strip's Staff ID | "Signed off — …" on screen and the byline re-read | **≤ 3 s** (Sign's, section 16) |
| See that a part's signature no longer counts | — | — | **0 acts** (the byline) |

Steps: seeing that a part's signature no longer counts paper *N* = 1 (find and read the annotation) → forced *M* =
0 → target *K* = 0; the real person re-signing 2 (read, sign) → 2 (*Sign off this note*, the strip's passphrase) →
2. Finding the parts to re-sign is chart by chart until slice 3c-3c's person menu (ADR-0093 (e)). The write half is
timed by the node (`gesture_kind = 'note-sign-off'`, section 5). **A figure outside its budget, or an `M > N`, is
the finding — file an issue; never adjust the budget.**

**Setup.** Section 23's setup (`nurse-a`, `dr-b`, the student `sam`, the section 2 chart, its `vouches` helper),
and section 16's superuser `boss`. Launch as section 3. Add:

```bash
conf() { psql "$CONN" -c "SELECT staff_id, cairn_key_confirmation(signing_key_id) FROM vault_member ORDER BY staff_id"; }
```

**Measurement 1 — the mark, and a colleague clears nothing.** As `nurse-a`: *New note*, `obs stable`, *Sign note*;
then *Add to this note*, `reviewed`, *Sign note* again, so the note has two parts. Then sign five more one-part notes
of hers, a second two-part note of hers alone (`fluids`, then `reviewed`), and a note two people write: she signs
`bp 120/80`, then `dr-b` presses *Add to this note*, types `agree`, *Sign note* — all for Measurement 2. As `boss`,
in *Superuser: enrollers and keys*, press *Repudiate key* on `nurse-a`'s row ("The key is repudiated. Give the member
a new key from the re-key form."). `conf` says `repudiated` for her. Re-key her from *Forgotten passphrase* with
`boss` in the strip (section 17): the same person, a new key; `conf` now says `confirmed` for her (a superuser's re-key is confirmed at once).

- [ ] Reopen the chart. Each of Nurse A's parts reads "signed by Nurse A (key had been repudiated) — counts as
      unsigned", then its time. Each of her two-part notes' state line reads "includes an unsigned part", and so
      does the note she and Dr B wrote. The old signature is still named: nothing is struck from the chart. *Sign
      off this note* is offered.
- [ ] As `dr-b`, press *Sign off this note* on the first two-part note (`obs stable`): "Signed off — every part of
      this note now carries your signature." Each part now reads "… — counts as unsigned · signed off by Dr B", the
      state line still reads "includes an unsigned part", and NOT "signed off by Dr B". `vouches` shows Dr B's rows
      as `sign-off`, **`attested`** (never `co-signed`).
- [ ] Dr B's button is still offered. Pressing it again answers "You have already signed every part of this note.
      Nothing more was signed." (ADR-0093 (c), #1008: record it, it is known).

**Measurement 2 — the real person re-signs (≤ 3 s).** As `nurse-a` on her NEW key.

- [ ] On the first two-part note she is offered *Sign off this note* and is NOT told "You have already signed every
      part …". After her press each part reads "signed by Nurse A (key had been repudiated) · signed off by Dr B ·
      re-signed by Nurse A", with no "— counts as unsigned", and the state line reads "signed off by Dr B". No
      dialog.
- [ ] On her second two-part note, which no colleague touched, her one press re-signs both parts: each reads
      "signed by Nurse A (key had been repudiated) · re-signed by Nurse A", and the state line reads "signed off by
      Nurse A" (ADR-0093 decision 10). `vouches` shows her rows as `sign-off`, `attested`.
- [ ] The known gap (ADR-0093 (h), #1011): on the note she and Dr B wrote, her press to re-sign her own part also
      vouches Dr B's. His part reads "signed by Dr B · co-signed by Nurse A", the state line reads "signed off by
      Nurse A", and `vouches` shows her row on Dr B's part as `co-signed`. On paper she would initial only her own
      entry. Record it.
- [ ] Time her press on the five one-part notes: from the first keystroke in the strip's Staff ID until "Signed off
      — …" shows and the note is re-read (budget ≤ 3 s), and `ui_gesture_timing`'s `note-sign-off` row. Each then
      reads "signed by Nurse A (key had been repudiated) · re-signed by Nurse A".
- [ ] A student: as `sam`, sign a note; repudiate and re-key `sam`. The part reads "signed by Sam (student) (key had
      been repudiated) — counts as unsigned", with no "— not co-signed" beside it. Note how the two parentheses read
      to a clinician. After Sam re-signs, it reads "… — not co-signed (no supervisor recorded) · re-signed by Sam (student)". Then `dr-b`
      presses *Sign off this note*: it reads "signed by Sam (student) (key had been repudiated) · re-signed by Sam
      (student) · co-signed by Dr B", the mark cleared, and `vouches` shows Dr B's row as `co-signed`.
- [ ] The known gap (ADR-0093 (d), #1010): on another of Sam's notes, `dr-b` signs off BEFORE Sam re-signs. After
      Sam's re-sign the part reads "— not co-signed (no supervisor recorded) · signed off by Dr B · re-signed by Sam
      (student)". Since slice 3c-3a *Sign off this note* stays offered: Dr B's press answers "You have already signed
      every part of this note. Nothing more was signed.", and another clinician's press co-signs the part and clears
      the mark (ADR-0094 (b)). Record it.

**Keyboard and screen reader, same pass as section 6** (VoiceOver on macOS, keyboard only):

- [ ] The byline's "(key had been repudiated) — counts as unsigned" is read with the note's heading, and so is each
      " · re-signed by …". The spoken form is the shown one (a claimed name still reads "unverified …").
- [ ] The state line "includes an unsigned part" is read before the text.
- [ ] *Sign off this note* is reached by Tab and read as "Sign off this note: {the note's time}".
- [ ] No dialog appears at any point.

**Chromium and WebKit.** The window is WebKit (Tauri on macOS). The slice changes no webview code, so compare the
words above with section 23's walk and note any difference in wrapping of the longer bylines.

**What could not be run here.** Every stopwatch figure and the VoiceOver pass need the real window. The rules and the
words were run as pure tests (`vouch_tests.rs`, `view_tests.rs`, `compound.rs`), and the flow through a throwaway
database by `note_repudiated_signer.rs` (repudiate and re-key, a colleague's sign-off attested and clearing nothing,
the real person's re-sign). The window's offer follows `needs_sign_off` (`notes_sign_off_tests.rs`). The real
window, the superuser panel's repudiate followed by a re-key, WebKit and VoiceOver were not seen. `TEMPLATE.md`
carries a matching block (*A repudiated signature*).

## 25. Supervision and the mandatory co-sign (GUI sign-in slice 3c-3a, ADR-0094)

The slice's gestures: a **supervision act** in the assigner's panel (*Supervision: who supervises whom*) or in *My
supervisees*, each with section 16's strip; and **Co-sign as reviewed** on a supervisee's note that owed no
co-signature. The chart's new words are: " — not co-signed" on the part of a writer the record requires to be
co-signed; " — not co-signed (no supervisor recorded)" when that writer, or a student, has no supervisor now; and,
in the assigner's panel, the gap row "needs co-signing · no supervisor recorded". Paper counterpart: a list of who
supervises whom in a staff file, a probationer's entries countersigned until the supervisors end the probation, and
a supervisor initialling a registrar's entry as read. **Live only**, like section 16.

**Budgets (the slice plan's §1.2).**

| Gesture | Start | Stop | Budget |
|---|---|---|---|
| A supervision act, credential included | the first keystroke in the strip's Staff ID | the act's sentence on screen and the list re-read | **≤ 3 s** (Sign's, section 16) |
| Co-sign as reviewed, credential included | the first keystroke in the strip's Staff ID | "Your review signature was recorded — …" on screen and the byline re-read | **≤ 3 s** (Sign's, section 16) |
| See that a part still needs its supervisor | — | — | **0 acts** (the byline) |

Steps: record "B supervises C" paper *N* = 1 (write it on the list) → forced *M* = 1 (one act, the strip's
passphrase counted with it) → target *K* = 1; set or lift co-signing 1 → 1 → 1; co-sign a reviewed note from the
opened chart 2 (open the folder, sign) → 2 (*Co-sign as reviewed*, the passphrase) → 2; see that a part still needs
its supervisor 1 (look for the countersignature) → 0 → 0. Finding what to co-sign is chart by chart until slice
3c-3c's person menu. The node times a review's write half under **`note-sign-off`**, the same kind as a sign-off
(ADR-0094 (f)), so take the review runs in a stretch with no sign-offs, or tell them apart by time. A supervision act
has no node timing: the stopwatch is its only figure. **A figure outside its budget, or an `M > N`, is the finding —
file an issue; never adjust the budget.**

**Setup.** Section 23's setup (`nurse-a`, `dr-b`, the student `sam`, the section 2 chart, its `vouches` helper),
section 16's superuser `boss` and section 24's `conf`. Enrol from *Enrol staff*, approved by `boss`, two clinicians:
`dr-p` ("Dr P", the probationer) and `dr-c` ("Dr C", nobody's supervisor). A superuser's approval confirms a key at
once; `conf` must say `confirmed` for everyone who presses a supervision button other than `boss` (a supervision act
needs a confirmed key, unless a superuser's). Launch as section 3. Add:

```bash
sup() { cairn-node --conn "$CONN" supervision-list; }   # one line per member, as the assigner's panel reads
```

**Measurement 1 — record supervision (≤ 3 s).** As `boss` in the strip, open *Supervision: who supervises whom*.
Time five acts, each from the first keystroke in the strip's Staff ID until the act's sentence shows and the list is
re-read.

- [ ] The panel lists every member but `boss`. Dr P's row reads "Dr P (dr-p): co-signing not required.". Sam's reads
      "Sam (sam): needs co-signing (student) · no supervisor recorded." (the gap row) and offers no *Require
      co-signing*: a student needs it by role.
- [ ] *Require co-signing* on Dr P's row: "dr-p now needs every signed note co-signed." Her row now reads "… needs
      co-signing · no supervisor recorded."
- [ ] Type `dr-b` in "Supervisor's staff ID for Dr P", press *Link a supervisor*: "dr-b now supervises dr-p." Her row
      reads "… needs co-signing · supervised by dr-b."
- [ ] *Grant assign-supervision right* on Dr P's row: "dr-p may now assign supervision while their own key is
      confirmed." Then, with `dr-p` in the strip, press *Unlink dr-b* on her own row: "The node refused this, so
      nothing was recorded. …" and the list is re-read; she still reads "supervised by dr-b". Nobody acts on their own
      supervision as the supervisee, even holding the right (ADR-0094 decision 2). As `boss`, *Withdraw
      assign-supervision right* on her row.
- [ ] With `dr-c` in the strip, *Link a supervisor* on Sam's row with `dr-c`: refused in the same sentence (`dr-c`
      holds no right). Then, with `boss`, link `dr-c` as Sam's supervisor and unlink again: Sam's row reads "… ·
      supervised by dr-c." then the gap row again.
- [ ] Every press clears the strip, refusals included. `sup` prints the same state as the panel. No dialog.

**Measurement 2 — the mark, and only a supervisor clears it.** Dr P needs co-signing and `dr-b` supervises her.

- [ ] As `dr-p`: *New note*, `obs stable`, *Sign note*. It reads "signed by Dr P — not co-signed · {its time}" and
      offers *Sign off this note*. Pressed by `dr-p` herself: "You have already signed every part of this note.
      Nothing more was signed." (ADR-0094 (i)).
- [ ] As `dr-c`: *Sign off this note*. The byline reads "signed by Dr P — not co-signed · co-signed by Dr C" and the
      button stays: Dr C is not her supervisor. `vouches` shows Dr C's row as `sign-off`, `co-signed`. Note how the
      success line "Signed off — every part of this note now carries your signature." reads beside a mark that
      stayed (the reading #1012 records for a repudiated part).
- [ ] As `dr-b`: *Sign off this note*. It reads "signed by Dr P · co-signed by Dr C · co-signed by Dr B", and the
      button is gone.
- [ ] As `dr-p`, sign a second note. Then, with `dr-b` named in who's here and in the strip, open *My supervisees*:
      Dr P's row offers *Lift co-signing* and *Step away*. Press *Lift co-signing*: "dr-p no longer needs notes
      co-signed. Notes signed while it was required still need a co-signature if they have none." The second note
      still reads "— not co-signed"; a note Dr P signs now reads "signed by Dr P" with no mark.
- [ ] The byline "(no supervisor recorded)": as `sam`, sign a new note. It reads "signed by Sam (student) — not
      co-signed (no supervisor recorded)". With nobody recorded, any non-student's sign-off clears it: `dr-c`'s press makes it
      "signed by Sam (student) · co-signed by Dr C".

**Measurement 3 — Co-sign as reviewed (≤ 3 s).** Dr P no longer needs co-signing; `dr-b` still supervises her. As
`dr-p`, sign five one-part notes, and one note that `nurse-a` then adds to (*Add to this note*, `agree`, *Sign note*).

- [ ] With `dr-c` named in who's here, none of these notes offers *Co-sign as reviewed*. With `dr-b` named, each of
      these notes offers it beside no *Sign off this note* (the five) and beside *Sign off this note* on the shared note.
- [ ] As `dr-b`, press it on the five, timing each from the first keystroke in the strip's Staff ID until
      "Your review signature was recorded — the chart shows which parts it covers." shows and the byline is re-read (budget
      ≤ 3 s); then `ui_gesture_timing`'s `note-sign-off` rows. Each reads "signed by Dr P · co-signed by Dr B", the
      button is gone, and `vouches` shows Dr B's row as `sign-off`, `co-signed`.
- [ ] On the shared note it co-signs Dr P's part only: Nurse A's part carries no "co-signed by Dr B", and `vouches`
      shows one new row.
- [ ] Key and claim differ: `dr-b` named in who's here, `dr-c` in the strip, on a fresh note of Dr P's: "There is
      nothing here for you to co-sign as reviewed. Nothing was signed." Nothing is written.
- [ ] A who's-here switch while a review is in flight: the picker says "Your review co-signature went through — open
      the chart to see which parts it covers."
- [ ] *Step away* in *My supervisees*: "dr-b no longer supervises dr-p."; the list reads "You supervise nobody
      here."; once the chart is read again, a fresh note of Dr P's no longer offers *Co-sign as reviewed* to `dr-b`. Close and reopen the assigner's panel (the
      panels do not refresh each other, ADR-0094 (g)): her row reads "Dr P (dr-p): co-signing not required."

**Keyboard and screen reader, same pass as section 6** (VoiceOver on macOS, keyboard only):

- [ ] Both panels' summaries are reached by Tab and open with Space or Enter. Each button is read with its whose-row
      name: "Require co-signing for Dr P", "Grant assign-supervision right for Dr P", "Unlink dr-b from Dr P", "Link a
      supervisor for Dr P", the field "Supervisor's staff ID for Dr P", "Lift co-signing for Dr P", "Step away from
      supervising Dr P". Enter in the link field does nothing (ADR-0094 (h)): press the button.
- [ ] Each act's sentence is read when it appears (the panels' status lines are live regions).
- [ ] *Co-sign as reviewed* is reached by Tab and read as "Co-sign as reviewed: {the note's time}".
- [ ] "— not co-signed (no supervisor recorded)" is read with the note's heading.
- [ ] No dialog appears at any point.

**Chromium and WebKit.** The window is WebKit (Tauri on macOS). This slice adds webview code (`supervision.js`, the
review button) that no headless walk has seen, in either engine. Note anything that differs from the words above,
and whether *My supervisees* still lists the previous person's supervisees after a who's-here switch (it is not
re-read until reopened, ADR-0094 (g)).

**What could not be run here.** Every stopwatch figure and the VoiceOver pass need the real window. The doors were run
against throwaway databases by `supervision_doors.rs`, `supervision_doors_approver.rs`, `supervision_admin_lib.rs` and
`restore_actor_registry_supervision.rs`; the record's effect on the chart by `note_supervision.rs`; the review by
`note_review_cosign.rs`. The window's commands were run by `cairn-gui-tauri`'s DB-gated tests (`notes_review_db_tests.rs`,
`supervision_panel_tests.rs`; CI never runs those, #905). The webview was checked by source guards only. The real
window, WebKit, Chromium and VoiceOver were not seen. `TEMPLATE.md` carries a matching block (*Supervision and the
mandatory co-sign*).

## 26. The person menu: "Waiting for you" (GUI sign-in slice 3c-3c, ADR-0095)

The slice's gestures: **"Waiting for you: N"** on the who's-here bar, a `<details>` that opens on three groups ("Your
signature", "Needs your co-signature", and the closed "Your supervisees' recent notes (last 7 days)"); **Open note**
on each row; and the picker's **" · N waiting"** after each person's entry. N counts the first two groups only.
Reading the menu signs nothing and asks for no passphrase. Paper counterpart: walking the ward or a stack of folders
to find the entries that wait for your signature or countersignature, and pulling a supervisee's files at random to
audit them. **Live only**: a `--mock` window draws an empty menu and no counts.

**Budgets (the slice plan's §1.2).**

| Gesture | Start | Stop | Budget |
|---|---|---|---|
| The menu read, 20 supervisees and a week of notes | the press on a name in the picker | "Waiting for you: N" on the bar | **≤ 1 s** |
| The picker's counts, same fixture | the picker's list on screen | the last " · N waiting" on screen | **≤ 1 s** |
| A pick while the picker's counts are still loading, same fixture | the press on a name in the picker, before the last " · N waiting" is on screen | the new name on the bar | **≤ 1 s** |
| Co-sign from the menu, credential included | the first keystroke in the strip's Staff ID, on the note "Open note" opened | "Signed off — …" or "Your review signature was recorded — …" and the byline re-read | **≤ 3 s** (Sign's, section 16) |

Steps: find what waits for my signature paper *N* = one walk per folder → forced *M* = 2 (open the menu, read it) →
target *K* = 1 (the bar's count); countersign one note from the menu 2 (open the folder, sign) → 3 (*Open note*,
*Sign off this note* or *Co-sign as reviewed*, the strip's passphrase) → 3, **ruled not an `M > N` defect**
(ADR-0095 decision 4: paper's "open the folder" includes finding it and walking to it); audit a supervisee's recent
work: pull files at random → 2 (open the menu, open the review group) → 2. Opening the menu shows the last read at
once and runs the same read again, so a pick, which runs it with a visible stop, is the stopwatch's view of "the menu
opens". The menu is a read, and the node times
only writes (section 5), so the stopwatch is the only figure for the first three rows; the co-sign's write half is
timed as **`note-sign-off`**. **A figure outside its budget, or an `M > N`, is the finding — file an issue; never
adjust the budget.** A miss on the first three rows is the case the query's `shortcut:` comment names: record it
on [#1021](https://github.com/cairn-ehr/cairn-ehr/issues/1021), the projection of open obligations ADR-0095
decision 1 names.

**Setup.** A FRESH node with section 25's Setup only, not its measurements: section 25's Measurement 2 leaves a
note of Dr P's without her supervisor's co-sign, which comes back into the lists below once `dr-b` and `dr-c` are
linked again, and every count below would then be one too many. From that Setup: the people (`nurse-a`, `dr-b`, the
student `sam`, `dr-p`, `dr-c`, `boss`), the section 2 chart (**chart A** below), `vouches`, `conf` and `sup`; also
section 20's `due` and `went` helpers, and one more chart, **chart B**. Launch as section 3. Put the supervision
record in this state at the console, then check it with `sup`:

```bash
cairn-node --conn "$CONN" supervision-set --op require-cosign --staff-id dr-p
cairn-node --conn "$CONN" supervision-set --op link-supervisor --staff-id dr-p --supervisor dr-b
cairn-node --conn "$CONN" supervision-set --op link-supervisor --staff-id dr-p --supervisor dr-c
cairn-node --conn "$CONN" supervision-set --op link-supervisor --staff-id nurse-a --supervisor dr-b
```

Dr P needs co-signing and has two supervisors; Nurse A is Dr B's supervisee and needs none.

**Measurement 1 — whose menu, what it counts, the three groups.**

- [ ] With nobody named ("Don't know" in the picker), the bar shows no "Waiting for you". Name `dr-c`: the bar reads
      "Waiting for you: 0". Opened, each of the first two groups reads "Nothing here.", and the third is a closed
      summary with no count.
- [ ] As `dr-p` on chart A: *New note*, `obs stable`, *Sign note*. Name `dr-b`: "Waiting for you: 1". Opened, under
      "Needs your co-signature": the patient, "Progress note · {its time}", "By Dr P — needs your co-signature (you or
      Dr C)", and *Open note*. Name `dr-c`: the same row reads "(you or Dr B)".
- [ ] As `nurse-a` on chart B: sign a one-part note. Name `dr-b`: still "Waiting for you: 1" (the review group is never
      counted). Open "Your supervisees' recent notes (last 7 days)": "By Nurse A — not yet reviewed". Name `dr-c`: no
      such row (Nurse A is not Dr C's supervisee).
- [ ] As `dr-b` on chart B: *New note*, type `plan`, and close the chart without signing. "Waiting for you: 2"; under
      "Your signature", first: "Draft note", and "goes into the record unsigned at {time} if nobody types in it".
- [ ] `cnode note-set-quarantine --minutes 2` (section 20) and leave the terminal alone with the menu CLOSED: once
      `went` lists the note, the bar is re-read within 60 s, and the row reads "Progress note" and "Went in unsigned —
      sign it", still counted. With the menu open the 60 s re-read is skipped (ADR-0095 (a)): reopen it to see the
      change. Put the quarantine back with `cnode note-set-quarantine --default`.
- [ ] With `dr-b` named, open *My supervisees* and press *Lift co-signing* on Dr P's row. Dr P's part from the second
      box is still under "Needs your co-signature" and still counted (a requirement lifted after the signing leaves
      the part waiting). As `dr-c`, *Sign off this note* on it: once the menu is read again (open it), the row has
      left Dr B's list too.
- [ ] Open "Who's here?": the list draws at once and the suffixes join it. Dr B's entry ends " · N waiting" with the
      N her bar shows once she is picked; an entry whose count is 0 has no suffix.

**Measurement 2 — Open note, and co-sign from it (≤ 3 s).** After Measurement 1, Dr B's list holds one COUNTED row,
her part that went in unsigned on chart B (her review group also holds Nurse A's note, which this measurement's
review row uses). Re-require co-signing for Dr P (the first console line above) and have her
sign **six** one-part notes on chart A, P1 to P6 in order. Name `dr-b`: "Waiting for you: 7".

- [ ] From the front door: *Open note* on P1's row. Chart A opens, scrolled to that note, which has the focus
      (VoiceOver reads it). The menu is closed.
- [ ] As `dr-b` on chart B: *New note*, type `later`. With the draft still being typed into, *Open note* on P2's row.
      The typing is saved (section 19's `parts`), the front door shows for a moment, then chart A opens on P2. Once
      the note pane draws, "Waiting for you: 8" (the new draft is hers).
- [ ] P1 to P5, one at a time: *Open note* on its row, press *Sign off this note* and time it, from the first
      keystroke in the strip's Staff ID until "Signed off — …" shows and the byline is re-read; then
      `ui_gesture_timing`'s `note-sign-off` rows. The bar's count drops by one each time once the note pane is
      re-read: 8 → 3 (the part gone in unsigned, the `later` draft, P6).
- [ ] The same from a review row: *Open note* on Nurse A's note from the review group, then *Co-sign as reviewed*
      (section 25 Measurement 3); the row leaves the review group, and the count stays 3.
- [ ] A row that vanished: open Dr B's menu on P6's row (her only row on chart A now), then at the console
      `cairn-node --conn "$CONN" supervision-set --op unlink-supervisor --staff-id dr-p --supervisor dr-b` and press
      *Open note* on that row. "That note no longer waits in your list — the list was read again." shows on the bar,
      with the focus on it; the list is redrawn without P6, "Waiting for you: 2"; no chart opens. Link her again
      afterwards (the second console line above): once the menu is read again, P6 is back and the count is 3.
- [ ] A drawn row of the previous person: with `dr-b`'s menu drawn, pick `dr-c` in who's here; the bar is re-read for
      Dr C at once, and no row of Dr B's stays on screen.

**Measurement 3 — the ≤ 1 s budgets, with 20 supervisees and a week of notes.** There is no seeding tool for this
fixture: enrol 20 clinicians from *Enrol staff* (section 17), link each to `dr-b` with the console line above, and
have each sign at least three notes across charts A and B, some under a requirement (so `dr-b`'s groups 2 and 3 both
hold rows). Record the fixture's real size in the template: the supervisees, `psql "$CONN" -Atc "SELECT count(*)
FROM clinical_note"` (the menu's query scans every row of it, ADR-0095 (h)), and how many rows `dr-b`'s menu shows.
A fixture smaller than the budget's is a figure for that size only: say so in the template.

- [ ] Time five picks of `dr-b`, from the press on her name in the picker to "Waiting for you: N" on the bar. Pick
      "Don't know" between runs: it hides the menu, so the bar's "Waiting for you: N" coming back is the stop (a pick
      of the person already named does not read the menu again, and another person's bar may show the same N).
- [ ] Time five openings of the picker, from its list on screen to the last " · N waiting".
- [ ] Time five picks made while the picker's counts are still loading: open the picker and press `dr-b`'s name
      before the last " · N waiting" shows, from the press to her name on the bar. Both reads use the window's one
      database connection, so the pick waits behind the counts' read (ADR-0095 (h)).

A fixture this size (about 60 notes) cannot show a cliff that grows with history: the menu's read still scans every
part its keys ever wrote (and the counts' read every person's), keeping only the last week's and those that could still
be owed, so a node with years of notes is slower than any figure here
([#1021](https://github.com/cairn-ehr/cairn-ehr/issues/1021)). The reads at scale (no window, no human) are
measured by `cargo run --release --example measure_person_menu` in `crates/cairn-node`; its figures are in
[`crates/cairn-node/results/2026-11-08-person-menu-reads-macos-m3max.md`](../../../crates/cairn-node/results/2026-11-08-person-menu-reads-macos-m3max.md).

**Keyboard and screen reader, same pass as section 6** (VoiceOver on macOS, keyboard only):

- [ ] "Waiting for you: N" is reached by Tab, opens with Space or Enter, and is read "Waiting for you: N — open the
      list".
- [ ] Each row's button is reached by Tab and read "Open note: {patient}, {when}"; its visible words are "Open note".
- [ ] The review group's summary is reached by Tab, read with its heading only (no count), and opens with Space or
      Enter.
- [ ] After *Open note*, the focus is on the note (or the draft) it named, and VoiceOver reads it.
- [ ] The refusal sentence is read when it appears, and has the focus.
- [ ] The counts failing: as the owner, `psql "$CONN" -c "REVOKE EXECUTE ON FUNCTION cairn_note_people() FROM
      cairn_window"`, open the picker: the list draws, its entries carry no suffix, and "Waiting counts could not be
      read." is read in the picker's status line. Undo it with `GRANT EXECUTE ON FUNCTION cairn_note_people() TO
      cairn_window`.
- [ ] No dialog, and no passphrase asked, at any point but the co-sign's strip.

**Chromium and WebKit.** The window is WebKit (Tauri on macOS). The webview was walked headlessly in Chromium only,
with a stubbed `invoke`: note anything in WebKit that differs from the words above, and whether the 60 s re-read ever
pulls the focus out of an open menu (it should not: that tick skips an open menu).

**What could not be run here.** Every stopwatch figure and the VoiceOver pass need the real window. The doors were run
against throwaway databases by `note_menu_doors.rs`; the cross-chart read by `note_parts_by_keys.rs`; the groups and
counts by `note_person_menu.rs`. The window's commands were run by `cairn-gui-tauri`'s DB-gated
`person_menu_db_tests.rs` (CI never runs those, #905). The webview was checked by source guards
(`person_menu_webview_tests.rs`) and a Chromium walk with a stubbed `invoke`. The real window, WebKit, the real Tauri
IPC and VoiceOver were not seen. `TEMPLATE.md` carries a matching block (*The person menu*).

## 27. The allergy list: the strip and its form (allergy slice 1, ADR-0096)

The slice's gestures: the **allergy strip**, one line in the patient identity header that is never blank; **Show all
allergies**, a `<details>` that opens the full lists, the two "none known" buttons and the form in place; the form's
**Sign**; and the strip's own **No known allergies** / **No known drug allergies (NKDA)** buttons, shown while nothing
has been asked. Paper counterpart: the allergy box on the drug chart and the red sticker on the folder: write
"Penicillin — anaphylaxis 2014", or "NKDA", and initial it. A `--mock` window reads "Allergies not yet asked" and
refuses every write ("fixture mode: …"), which is enough for the labels and the Tab order; everything else is live
only.

**Budgets (the slice plan's §1.2, design §6).**

| Gesture | Start | Stop | Budget |
|---|---|---|---|
| A free-text entry, the form already open | the first keystroke in *Substance* | "Recorded." on the allergy status line and the strip redrawn | **≤ 10 s** |
| "None known", from the strip | the press on *No known drug allergies (NKDA)* | the strip reads "NKDA (asked …)" | **≤ 3 s** |
| Reading whether a dangerous allergy is recorded | the chart on screen | the strip read | no click |

The budgets are the plan's gesture, **with the key held** (`--key-hold-seconds 900`, one passphrase first). Time
each gesture again on a shared terminal (no hold: the staff ID and passphrase typed in the signer strip before the
press, the first keystroke there starting the clock) and record both. Steps: read the allergies paper *N* = 0 →
forced *M* = 0 → target *K* = 0; record an entry 1 → 1 → 1 (the form and Sign); "none known" 1 → 1 → 1 press. The
node times the write half of each as **`allergy-record`** and **`allergy-none`** (section 5). **A figure outside its
budget, or an `M > N`, is the finding — file an issue; never adjust the budget.**

**Setup.** Sections 0 and 2, then section 16's Setup and its Measurement 2 (the vault, the terminal passphrase, and
`nurse-a` enrolled from *Enrol staff*: any clinician records an allergy, nurses included). Nothing in this slice can
be withdrawn (ADR-0096 (b)), so every state below needs a fresh chart. Register them with the owner's connection,
each with its own name and date of birth:

```bash
cnode() { cairn-node --conn "$CONN" --key "$NODE_KEY" "$@"; }   # a function: an unquoted $CONN would word-split
achart() { cnode patient-register --name "$1" --birth-date "$2" --confirm-new | sed -n 's/^registered patient //p'; }
A1=$(achart "Allergy Alpha" 1961-01-01)   # not yet asked → NKDA → NKDA + latex
A2=$(achart "Allergy Bravo" 1962-02-02)   # no known allergies → penicillin + an adverse effect
N1=$(achart "Nkda One" 1971-03-01); N2=$(achart "Nkda Two" 1972-03-02); N3=$(achart "Nkda Three" 1973-03-03)
N4=$(achart "Nkda Four" 1974-03-04); N5=$(achart "Nkda Five" 1975-03-05)
for c in "$A1" "$A2" "$N1" "$N2" "$N3" "$N4" "$N5"; do [ -n "$c" ] || echo "a registration failed — read the output above" >&2; done
echo "A1=$A1 A2=$A2"
```

Launch as section 3 on `$A1`, first with `--key-hold-seconds 900`, and unlock the terminal (section 16 Measurement
1). Open the other charts with *Find another patient* and a search by name (section 8), never by relaunching
between runs of one gesture.

**Measurement 1 — the five strip states, read with no click.** Today's date reads in the form "9 Nov 2026".

- [ ] `$A1` opened: the strip reads **"Allergies not yet asked"**, not prominent, with *No known allergies* and *No
      known drug allergies (NKDA)* beside it. While the chart is read it says "Reading the allergy list…", never a
      blank line.
- [ ] As `nurse-a`, press *No known drug allergies (NKDA)* on the strip. The strip reads **"NKDA (asked {today}) ·
      other allergies: not yet asked"**, its two buttons are gone, and "Recorded." shows on the allergy status line, under *Show all allergies*.
- [ ] *Show all allergies*, then the form: *Other substance*, *Substance* `latex`, *Allergy*, *Urticaria (hives)*,
      *Seen documented*, *Sign*. The strip reads **"NKDA (asked {today}) · Other: latex → urticaria"** (the "Allergy: "
      prefix dropped under "Other:"), now prominent: the warning colour, the bold weight and a bar at its left edge.
- [ ] `$A2`: press *No known allergies* on the strip. It reads **"No known allergies (asked {today})"**; *Show all
      allergies* opens on "No known allergies — asked {today} by {nurse-a's display name}".
- [ ] On `$A2`, the timed entry below (penicillin) and then an adverse effect: *Drug*, `metformin`, *Adverse
      effect*, *GI upset*, "What happened, in words" `loose bowel motions`, *When* `2019`, *Patient stated*,
      *Sign*. The strip reads **"Allergy: penicillin → anaphylaxis · 2014 · no known other allergies (asked {today})
      · +1 adverse effect"**: the drug allergy overrides "no known allergies" in its own list only, with no flag
      (ADR-0096 decision 13), and the adverse effect is counted, never named, on the strip. Opened, the lists read
      "No known other allergies — asked {today} by {nurse-a's display name}", "Drug allergies:" with the penicillin
      line (prominent), and "Adverse effects:" with "Previous adverse effect (2019): metformin → GI upset: loose bowel
      motions (patient stated)" (quiet); no "Previously recorded" line (the statement still stands for other
      allergies); each entry line ends " · recorded {YYYY-MM-DD} by {nurse-a's display name}".
- [ ] The wire words as signed, at the console: `psql "$CONN" -c "SELECT term, allergen_kind, kind,
      reaction_category, reaction_text, onset_precision, onset_value, source FROM clinical_allergy WHERE patient_id IN
      ('$A1', '$A2') ORDER BY hlc_wall"` shows latex / other / allergy / urticaria / (empty) / not-known / (empty) /
      seen-documented, penicillin / drug / allergy / anaphylaxis / (empty) / year / 2014 / seen-documented, and
      metformin / drug / adverse-effect / gi-upset / loose bowel motions / year / 2019 / patient-stated.

**Measurement 2 — the timings (five runs each, held key, then shared terminal).**

- [ ] **A free-text entry (≤ 10 s).** On `$A2` (the first run is Measurement 1's penicillin), the form open: start at
      the first keystroke in *Substance*; type `penicillin`, leave *Drug*, press *Allergy*, *Anaphylaxis*, type `2014`
      in *When*, press *Seen documented*, then *Sign*; stop when "Recorded." shows and the strip is redrawn. Later
      runs add `amoxicillin`, `cefalexin`, `trimethoprim`, `codeine` the same way, so the strip lists them newest first
      (all are anaphylaxis), joined with "; ".
- [ ] **"None known" (≤ 3 s).** On `$N1` to `$N5`, one each: from the press on the strip's *No known drug allergies
      (NKDA)* to "NKDA (asked {today})" on the strip.
- [ ] Then the write halves: `psql "$CONN" -c "SELECT * FROM ui_gesture_timing WHERE gesture_kind LIKE 'allergy-%'"`
      has `allergy-record` and `allergy-none` rows in bucket `1-3`.
- [ ] Relaunch with no `--key-hold-seconds` (a shared terminal) on fresh charts (register more with `achart`) and
      repeat both, the clock starting at the first keystroke in the signer strip's *Staff ID*.

**Measurement 3 — the refusals, before the key.** On any chart, the form open:

- [ ] A *Substance* typed and nothing pressed for "What kind": *Sign* answers **"Choose Allergy or Adverse effect."**
      under the "What kind:" row, and nothing is signed. The form never starts with *Allergy* or *Adverse effect* pressed, nor any
      "What happened" button, on load or after a signed entry.
- [ ] An empty *Substance*: "Name the substance." under it (said first, whatever else is missing). A kind pressed
      and nothing for "What happened": "Choose what happened, or Not known." under that row.
- [ ] *When* `31/02/2019`: "\"31/02/2019\" is not a date this window can read — type 2019, 03/2019 or 14/03/2019, or
      leave it empty if it is not known." under *When*. Next year (e.g. `2027`): "\"2027\" is after today — when it
      happened cannot be later than today." This month as `MM/YYYY` is accepted.
- [ ] After each refusal, what was typed stays, and the passphrase field is empty (ADR-0096 (g)): record each
      retype on a shared terminal as a paper-parity cost.
- [ ] Enter in a text field signs nothing; the only act is *Sign*.

**Measurement 4 — the real Tauri IPC** (the mock walk stubbed `invoke`, so it could not prove the argument names).
The webview sends `patientId`, `charts`, `entry`, `scope` and `credential`, and Rust takes `patient_id`; the entry's
own fields are snake_case (`allergen_kind`, `reaction_text`). A name Tauri cannot map fails the call with an
"invalid args" error.

- [ ] The strip reads a state of Measurement 1, never "Could not read the allergy list: invalid args …"
      (`allergy_list`).
- [ ] "Recorded." after an entry and after a "none known" press, and the console row of Measurement 1 shows every
      field as typed (`allergy_record`'s `entry`, `allergy_none`'s `scope`, `credential`).

**Measurement 5 — a chart left mid-act.** On a shared terminal, type the credential and an entry on `$A1`, press
*Sign* and at once *Find another patient*, and open `$A2` before the answer lands (a slow node helps; skip the
check if the answer always beats the click, and say so). `$A2`'s strip reads its own state, never "Reading…" stuck,
and nothing of `$A1`'s outcome shows on `$A2` (ADR-0096 decision 19). Reopen `$A1`: the entry is there.

**A chart whose link is in doubt (optional, section 13 Setup A).** If you have that pair, record an allergy on one
chart and open the other: the line is shown, on the strip and in the list, ending "(on a chart whose link to this
one is in doubt)", with a dashed rule beside it in the list. It is never withheld. A "none known" statement on
the doubted chart is the opposite: on the other chart the strip still reads "Allergies not yet asked" (with its
two buttons), and the statement shows only under "Previously recorded", ending with the same mark.

**Keyboard and screen reader, same pass as section 6** (VoiceOver on macOS, keyboard only):

- [ ] The strip is read with the identity header, as one line, and its new text is announced after a sign without
      taking the focus.
- [ ] *Show all allergies* is reached by Tab and opens with Space or Enter.
- [ ] Each toggle row is read with its label ("Allergen:", "What kind:", "What happened:", "Source:"), and each
      button with its pressed state ("selected" or not). A pressed toggle is visible without colour (its outline and
      weight change).
- [ ] *Substance*, "What happened, in words (optional)" and "When — leave empty if not known" are read by their
      labels, and *When*'s placeholder "2019 · 03/2019 · 14/03/2019" is read or visible.
- [ ] A field refusal is read when it appears, and again with its field when the focus reaches it (each error
      describes its control).
- [ ] An allergy line is distinguishable from an adverse effect without colour: its text says "Allergy" or "Patient
      stated allergy", the other "Previous adverse effect".
- [ ] No dialog at any point; the passphrase is asked only by the signer strip.

**WebKit.** The window is WebKit (Tauri on macOS). The webview was walked headlessly in Chromium and in Playwright's
WebKit with a stubbed `invoke`, never in the real window: note anything that differs from the words above, in
particular the toggles' `aria-pressed` reading and the `<details>` opening by keyboard.

**What could not be run here.** Every stopwatch figure, the VoiceOver pass, the real Tauri IPC and the real window's
WebKit need the window and a human. The floor was run by `db/tests/074_clinical_allergy_test.sql` and
`allergy_floor.rs`; the signing and the read by `allergy_sign.rs`, `allergy_read.rs` and `window_role_grants.rs`; the
strip's wording by `allergy/view_tests.rs`. The window's commands were run by `cairn-gui-tauri`'s DB-gated
`allergies_tests.rs` (CI never runs those, #905); the webview by source guards (`allergies_webview_tests.rs`) and
the stubbed walk. An outcome that is not known (#814) and a re-read that fails after a sign cannot be produced by
hand here; they are pinned by `allergies_tests.rs`'s `an_interrupted_sign_is_not_known_and_never_retried` and
`a_landed_sign_is_an_ok_with_or_without_its_pane`. `TEMPLATE.md` carries a matching block (*The allergy list*).

## 28. Correct, de-label and entered in error on the allergy list (allergy slice 2, ADR-0097)

The slice's gestures, all in the expanded list (*Show all allergies*): **Correct** on a live line, which fills the
entry form from the line as recorded and says "Correcting: …" above it, then the form's *Sign*; **De-label** on a live
line, which opens one inline row under it (*Tolerated*, *When*, *Note (optional)*, *Sign*, *Cancel*); **Entered in
error** on a live line, a historical line or a "none known" line, an inline row (*Reason (optional)*, *Sign*,
*Cancel*); and **Withdraw de-label** on a historical line, the same row aimed at the de-label it rests on. A line from a
chart whose link to this one is in doubt carries no button (ADR-0097 decision 25). The chart's new words: "Earlier: …"
under a corrected line, "Historically reported: … Challenged: tolerated … (de-labelled {day} by {name})", "Entered in
error: …" (with the reason typed, " — {reason}"), "Not counted: …", " — differing accounts", and the byline markers
" (signer unconfirmed)" / " (key had been repudiated)". Paper
counterpart: strike the line in the drug chart's allergy box and write the corrected one beside it, initialled; write
"tolerated amoxil 03/22" against a struck penicillin line; strike a slip and initial it. **Live only**: a `--mock`
window refuses every write ("fixture mode: …"), which is enough for the button labels and the Tab order.

**Budgets (the slice plan's §1.2).**

| Gesture | Start | Stop | Budget |
|---|---|---|---|
| Correct a line's reaction | the press on the line's *Correct* | "Recorded." on the allergy status line and the list redrawn, the old line under "Earlier:" | **≤ 15 s** |
| De-label a line | the press on the line's *De-label* | "Recorded." and the line under "Historically reported (de-labelled):" | **≤ 15 s** |
| Entered in error | the press on the line's *Entered in error* | "Recorded." and the line under "Entered in error:" | **≤ 5 s** |
| See that two accounts of one allergen differ | the chart on screen | the strip read | no click |

The budgets are the plan's gesture, **with the key held** (`--key-hold-seconds 900`, one passphrase first); time each
again on a shared terminal (no hold, the clock starting at the first keystroke in the signer strip's *Staff ID*) and
record both. Steps: correct paper *N* = 1 (strike and rewrite) → forced *M* = 1 (one entry carrying `correction-of`) →
target *K* = 1 (Correct, edit, Sign); de-label 1 → 1 → 1 (De-label, name what was tolerated, Sign); entered in error
1 → 1 → 1 (Entered in error, Sign); strike a de-labelled line 1 → 1 → 1 (*Entered in error* on the historical line,
Sign — aimed at its entry, never through *Withdraw de-label*). The node times the write half of
each as **`allergy-correct`**, **`allergy-challenge`** and **`allergy-retract`** (section 5). **A figure outside its
budget, or an `M > N`, is the finding — file an issue; never adjust the budget.**

**Setup.** Section 27's setup (sections 0 and 2, section 16's Setup and Measurement 2, `nurse-a`, its `cnode` and
`achart` functions), and section 19's `dr-b` (enrolled from the window, so with a passphrase). Section 1's
`/tmp/dr-a.key` links two charts. Register:

```bash
O1=$(achart "Overlay One" 1981-01-01)      # Correct, five timed runs
O2=$(achart "Overlay Two" 1982-02-02)      # De-label, five timed runs
O3=$(achart "Overlay Three" 1983-03-03)    # Entered in error, five timed runs
W1=$(achart "Overlay Words" 1984-04-04)    # Measurement 1: the words
L1=$(achart "Overlay Linked" 1985-05-05); L2=$(achart "Overlay Linked" 1985-05-05)
for c in "$O1" "$O2" "$O3" "$W1" "$L1" "$L2"; do [ -n "$c" ] || echo "a registration failed — read the output above" >&2; done
cnode link-charts "$L1" "$L2" --attester-key /tmp/dr-a.key
```

On each of `$O1`, `$O2` and `$O3`, record five drug allergies through the form as section 27 Measurement 2 does
(untimed): `penicillin`, `amoxicillin`, `cefalexin`, `trimethoprim`, `codeine`, each *Allergy*, *Rash*, *Seen
documented*. Launch as section 3 on `$W1` with `--key-hold-seconds 900`, and unlock the terminal.

**Measurement 1 — the words (on `$W1`).** Today reads "9 Nov 2026"; `{name}` is the signer's display name.

- [ ] Record `penicillin`, *Allergy*, *Rash*, *Seen documented* (every entry below is *Seen documented*, so its
      line starts "Allergy:"). *Correct* on its line: the form is filled with penicillin / Drug /
      Allergy / Rash / Seen documented, and "Correcting: Allergy: penicillin → rash" shows above it. Press
      *Anaphylaxis*, *Sign*. The list reads "Allergy: penicillin → anaphylaxis" with, under it, muted, "Earlier:
      Allergy: penicillin → rash (recorded {today} by {name})"; the strip reads "Allergy: penicillin → anaphylaxis";
      the form is empty and "Correcting" is gone.
- [ ] *De-label* on that line: one row opens under it, *Tolerated* EMPTY (*Sign* now: "Name what was tolerated." in
      the row). Type `amoxicillin`, *When* `03/2022`, *Sign*. The line moves under "Historically reported
      (de-labelled):" and reads "Historically reported: penicillin allergy. Challenged: tolerated amoxicillin Mar 2022
      (de-labelled {today} by {name})", muted, carrying *Withdraw de-label* and *Entered in error*; the strip reads
      "Allergies not yet asked · +1 historical" and shows the two "none known" buttons.
- [ ] *Withdraw de-label* on the historical line, *Sign*: the anaphylaxis is back on the strip, prominent, and the
      list shows "Entered in error: de-label — tolerated amoxicillin (withdrawn {today} by {name})" under "Entered in
      error:".
- [ ] *Entered in error* on the live anaphylaxis line: its row opens with "This strikes the line and its earlier
      versions. To undo a correction, press Correct and record it as it was." above *Reason* (the line has an
      "Earlier:" version; on a line with none, the row has no such sentence). *Reason* `wrong patient`, *Sign*: the
      strip reads "Allergies not yet asked"; the list reads "Entered in error: Allergy: penicillin → anaphylaxis
      (withdrawn {today} by {name}) — wrong patient", with the "Earlier:" rash line under it. No "Entered in error"
      line carries a button (a withdrawal is never withdrawn, ADR-0097 (a)).
- [ ] Strike a de-labelled line in one act: record `codeine`, *Allergy*, *Rash*; *De-label* it (`codeine`, *When*
      empty, *Sign*); then *Entered in error* on its historical line, *Reason* `wrong patient`, *Sign*. The strip
      never shows codeine between the two acts; the list reads "Entered in error: Allergy: codeine → rash (withdrawn
      {today} by {name}) — wrong patient — de-labelled {today} by {name}: tolerated codeine", and the historical
      heading is gone.
- [ ] Differing accounts: record `penicillin`, *Allergy*, *Rash*, then a second entry `Penicillin`, *Allergy*,
      *Anaphylaxis*. The strip reads "Allergy: Penicillin → anaphylaxis — differing accounts; Allergy: penicillin →
      rash — differing accounts", with no click. Record `penicillin`, *Allergy*, *Not known*: that line carries no
      mark ("not known" never differs).
- [ ] *No known drug allergies (NKDA)* in the list (`$W1`'s drug allergies override it, so it is listed under
      "Previously recorded:"), then *Entered in error* on that line, *Sign*: it reads "Entered in error: no known drug
      allergies (asked {today} by {name})", and "Previously recorded:" no longer lists it.
- [ ] As `nurse-a`, *Entered in error* on a line `dr-b` signed: it signs (a nurse may strike a doctor's line, as on
      paper; ADR-0097 decision 2).
- [ ] Each history list sits under its own heading, never under "Other allergies:".

**Measurement 2 — a linked chart's line (ADR-0097 decision 8).** Open `$L2` and record `latex`, *Other substance*,
*Allergy*, *Urticaria*. Open `$L1`: the latex line is listed. *Correct* it to *Angioedema* and *Sign*: "Recorded.",
and `$L1`'s list shows the corrected line. Then:

- [ ] `psql "$CONN" -c "SELECT patient_id, term, reaction_category, refs FROM clinical_allergy WHERE term = 'latex'
      ORDER BY hlc_wall"` shows both rows with `patient_id` = `$L2`, the second with `refs` =
      `[{"rel": "correction-of", "event": "<the first row's event_id>"}]`.
- [ ] *De-label* and *Entered in error* on a linked chart's line land on its chart the same way
      (`clinical_allergy_challenge`, `clinical_allergy_retraction`, column `patient_id`).
- [ ] **A doubted link (ADR-0097 decision 25).** With section 13's Setup B (A–C separated, still joined through B;
      `cairn_chart_set_doubts` prints its `separated` row), record `latex`, *Other substance*, *Allergy*, *Urticaria*
      on `$C`, and open `$A`: the latex line reads "… (on a chart whose link to this one is in doubt)" and carries NO
      button (no *Correct*, *De-label* or *Entered in error*). Open `$C`: there its own line carries all three.

**Measurement 3 — the timings (five runs each, held key, then shared terminal).**

- [ ] **Correct (≤ 15 s).** On `$O1`, one line per run: from the press on *Correct* to "Recorded." and the redraw,
      pressing *Anaphylaxis* in place of *Rash* and *Sign*.
- [ ] **De-label (≤ 15 s).** On `$O2`, one line per run: from the press on *De-label*, typing `amoxicillin` in the
      empty *Tolerated* and `03/2022` in *When*, *Sign*, to "Recorded." and the line under the historical heading.
- [ ] **Entered in error (≤ 5 s).** On `$O3`, one line per run: from the press on *Entered in error*, *Sign* with no
      reason, to "Recorded." and the line under "Entered in error:".
- [ ] The write halves: `psql "$CONN" -c "SELECT * FROM ui_gesture_timing WHERE gesture_kind IN ('allergy-correct',
      'allergy-challenge', 'allergy-retract')"` has a row for each, bucket `1-3`. A correction never adds to
      `allergy-record`.
- [ ] Relaunch with no `--key-hold-seconds` on fresh charts (register and seed more with `achart` and the form) and
      repeat all three.

**Measurement 4 — the refusals, before the key.**

- [ ] *De-label*, *Tolerated* left empty, *Sign*: "Name what was tolerated." in the row (not under the form's
      *Substance*), what was typed kept. *When* `2999`: "\"2999\" is after today — when it happened cannot be later than
      today." in the row.
- [ ] Correct on a line, then empty *Substance* and *Sign*: "Name the substance." under it.
- [ ] *Cancel* and Escape close a row signing nothing; Escape returns the focus to the line's first button. Neither
      closes the row while a *Sign* is in flight.
- [ ] **A line changed on another terminal (two terminals, as section 23).** Both windows on one chart, both lists
      drawn. On terminal A, *Entered in error* on a line and *Sign*. On terminal B, without reading the chart again,
      *De-label* that line and *Sign*: "That allergy line is no longer on this chart — read it again.", and the list
      is read again. Nothing is signed (`clinical_allergy_challenge` has no new row).
- [ ] **The suspended correction (R-6a).** Terminal B: *Correct* on a live line and change its reaction, leaving the
      form unsigned. Terminal A: *Entered in error* on that line. Terminal B: *Sign*. The window reads the list again;
      the form keeps what was typed, *Sign* is disabled, and "The line you were correcting has changed on the record,
      or its chart's link came into doubt — press Correct on its current line if it has one, or Cancel correction."
      replaces "Correcting: …". Pressing *Sign* again sends
      nothing. *Cancel correction* empties the form and *Sign* comes back.
- [ ] Skip the two-terminal checks if a second window cannot be run against this node, and say so.

**Measurement 5 — the real Tauri IPC** (the walk stubbed `invoke`, so it could not prove the argument names). The
webview sends `patientId`, `charts`, `target`, `tolerated`, `date`, `text` and `credential`, and the entry's own
snake_case `corrects`; a name Tauri cannot map fails the call with an "invalid args" error.

- [ ] "Recorded." after a correction, a de-label, a withdrawal of a line, of a "none known" statement and of a
      de-label, never "invalid args" (`allergy_record` with `entry.corrects`, `allergy_challenge`, `allergy_retract`).
- [ ] The console rows of Measurements 1 and 2 show each field as typed: `tolerated_term`, `date_precision` /
      `date_value` (`month` / `2022-03`), `text` (`wrong patient`, and `NULL` where nothing was typed).

**The byline markers (optional, with section 24's setup and its `conf` helper).**

- [ ] As `nurse-a`, record `penicillin`, *Allergy*, *Anaphylaxis* on a fresh chart, then *Entered in error* on it.
      As `boss`, repudiate `nurse-a`'s key (section 24 Measurement 1). Reopen the chart: the anaphylaxis is back on the
      strip; the list reads "Not counted: entered in error by {Nurse A's name} (key had been repudiated)" under "Not
      counted:", and the line's byline ends " by {Nurse A's name} (key had been repudiated)". A repudiated key takes
      no warning away (ADR-0097 decision 11).
- [ ] A signer whose key `conf` reports `unconfirmed` (section 23's Measurement 3 member): her lines' bylines end
      " (signer unconfirmed)", and her *Entered in error* counts.

**Keyboard and screen reader, same pass as section 6** (VoiceOver on macOS, keyboard only):

- [ ] Each line's buttons are reached by Tab and read with the line: "Correct: Allergy: penicillin → rash …",
      "De-label: …", "Entered in error: …", "Withdraw de-label: Historically reported: …", "Entered in error:
      Historically reported: …".
- [ ] Whether the retract row's "This strikes the line and its earlier versions. …" is read when the row opens (it
      is a paragraph before the focused *Reason* field, not its description).
- [ ] An inline row is read as a group named for its button and line, its fields by their labels (*Tolerated*, "When —
      leave empty if not known", *Note (optional)*, *Reason (optional)*); the focus lands in its first field.
- [ ] A refusal in a row is read when it appears.
- [ ] Note whether "Correcting: …" is read when *Correct* moves the focus to *Substance* (it is a paragraph above the
      form, not the field's description), and whether the suspension sentence is announced (it is not an alert).
- [ ] Each history list is read with its heading ("Historically reported (de-labelled):", "Entered in error:", "Not
      counted:", "Previously recorded:").
- [ ] A historical or entered-in-error line is distinguishable from a live allergy without colour: its text starts
      "Historically reported:" or "Entered in error:".
- [ ] No dialog at any point; the passphrase is asked only by the signer strip.

**WebKit.** The window is WebKit (Tauri on macOS). The webview was walked headlessly in Chromium and Playwright's
WebKit with a stubbed `invoke`, never in the real window: walk each act above in the real window, including the
suspended correction, and note anything that differs from the words above, in particular the inline row's focus,
Escape, and the muted nested "Earlier:" lists.

**What could not be run here.** Every stopwatch figure, the VoiceOver pass, the real Tauri IPC, the two-terminal
checks and the real window's WebKit need the window and a human. The floor was run by
`db/tests/075_allergy_overlays_test.sql` and `allergy_overlay_floor.rs`; signing and reading by `allergy_overlays.rs`
and `window_role_grants.rs`; the fold and its words by `allergy/thread_tests.rs`, `allergy/view_overlay_tests.rs` and
`allergy/view_history_tests.rs`. The window's commands were run by `cairn-gui-tauri`'s DB-gated
`allergy_overlays_tests.rs` (CI never runs those, #905), the target rule also by the pure
`allergy_overlays_pure_tests.rs`; the webview by source guards (`allergy_overlays_webview_tests.rs`) and the stubbed
walk. An outcome that is not known (#814) is pinned by `allergy_overlays_pure_tests.rs`'s
`an_interrupted_overlay_is_not_known_and_never_retried`.
`TEMPLATE.md` carries a matching block (*Allergy overlays*).

## 29. A drug allergy coded against drugref, and the medication check (allergy slice 3, ADR-0098)

The slice's gestures:
- **The pick.** While the allergen is *Drug*, three letters in *Substance* list drugs and drugref allergy groups under
  the field (a group reads "{name} (group)", unless its name already says "group": the real "Penicillins (allergy group)"
  reads as it is). Choosing one codes the entry. Typing on, or pressing *Other substance*,
  drops the coding, and the entry is then recorded as typed.
- **The check.** Every current line of the medication list is read against the live drug allergies on every read:
  - a hit carries the loud **Allergy** tag beside the drug's name and its words on a row under the line;
  - a previous adverse effect is a muted row;
  - a line some live allergy could not be verified against carries the small **not checked** tag and no words.
- **The review status.** A group that is not clinically reviewed is said in the expanded allergy list only:
  " (drug group not clinically reviewed)".

Paper counterpart: write "Penicillin — anaphylaxis" in the drug chart's allergy box. The prescriber or pharmacist then
reads that box against every line of the drug chart before a dose is given. **Live only**: a `--mock` window has no
drug list (the pick shows its hint) and no allergies.

**Budgets (the slice plan's §1.2).**

| Gesture | Start | Stop | Budget |
|---|---|---|---|
| A coded entry, the form open | the first keystroke in *Substance* | "Recorded." and the medication list's tags redrawn | **≤ 15 s** |
| A free-text entry, the pick present | the first keystroke in *Substance* | "Recorded." | **≤ 10 s** |
| The pick appears | the third letter typed | the option list drawn | **≤ 300 ms** |
| Reading a hit on a medication line | the list on screen | the tag and its words read | no click, no hover |
| Telling an unchecked line from a checked one | the list on screen | read | at a glance |
| The medication pane's open, drugref present vs absent | the chart opened | the list drawn | **+ ≤ 100 ms** |

The budgets are the plan's gestures, **with the key held** (`--key-hold-seconds 900`). Time the two entries again on
a shared terminal (no hold, the clock starting at the first keystroke in the signer strip's *Staff ID*) and record
both. Steps:
- record a coded entry: paper *N* = 1 (write the line) → forced *M* = 1 (one signed event; the coding rides on it) →
  target *K* = 1 (type, choose or keep typing, Sign);
- check the list: 1 (read the box against the chart) → 0 → 0, computed on every read.

The node times the write half of an entry as **`allergy-record`** (section 5), coded or not. **A figure outside its
budget, or an `M > N`, is the finding: file an issue, and never adjust the budget.**

**Setup A: a drugref WITH its allergy groups.** The check needs drugref's `allergy_group` table and its
`allergy_check` function (drugref db/067–069, drugref PR #225). Which local database to copy it from (all on :5532,
checked 2026-11-10):

| Database | Has the allergy groups? | Use it? |
|---|---|---|
| `drugref_allergy` | Yes: built by drugref's allergy-groups round, at drugref db/069, groups ingested (MeSH 2026). One group, "Penicillins (allergy group)", `structural-seed`. Its `allergy_check` result shape matches db/076's string exactly. | **Yes, the source.** drugref's notes say never patch it, so only `pg_dump` from it. |
| `drugref_profiles` | **No**: it is at drugref db/065. | No: Cairn would read it as "no drug list". |
| `drugref_test` | It has db/067–069 but no groups, and drugref's pytest recreates it. | Never: it is not a source. |

If `drugref_allergy` is gone, build a new one from the drugref repository, as its own HANDOVER describes:
1. `createdb -T drugref_profiles drugref_allergy`;
2. migrate it to db/069;
3. `drugref ingest allergy-groups --release 2026 --desc … --supp …` (the MeSH 2026 descriptor and supplementary files).

Then check the source before copying it. The query below must print exactly the line under it; any other shape reads
as "no drug list" on purpose (ADR-0098 decision 14), and is a finding against db/076's shape string, not something to
patch around:

```bash
psql -h 127.0.0.1 -p 5532 -d drugref_allergy -Atc "SELECT pg_get_function_result('drugref.allergy_check(uuid,uuid)'::regprocedure)"
# TABLE(outcome text, match_kind text, reason text, group_class_uuid uuid, group_name text, group_mesh_ui text, review_status text, allergen_component uuid, prescribed_component uuid)
```

**Install it into the rig's Cairn database as the schema `drugref`, OWNED BY THE OWNER** (ADR-0098 decision 13):
whoever can replace `drugref.allergy_check` runs SQL with the door owner's rights every time the window reads a
medication list. Build this checkout first. Then let any owner `cnode` command load db/076 (every owner connect
replays the schema), and re-run section 0's `provision-window-role`, because the build moves `SCHEMA_GENERATION` to
76. Use the PG18 `pg_dump` from `$PGBIN`, never the one on `PATH` (on the maintainer's Mac that is PG16, which refuses a
PG18 server):

```bash
"$PGBIN/pg_dump" -h 127.0.0.1 -p 5532 -d drugref_allergy -n drugref --no-owner --no-privileges \
  | "$PGBIN/psql" "$CONN" -v ON_ERROR_STOP=1 -q
psql "$CONN" -Atc "SELECT nspowner::regrole FROM pg_namespace WHERE nspname = 'drugref'"   # the owner ($USER)
psql "$CONN" -Atc "SELECT cairn_drug_available()"                                          # t
psql "$WINDOW_CONN" -Atc "SELECT count(*) FROM cairn_drug_search('amox', 12)"              # ≥ 1: the window may call the doors
psql "$WINDOW_CONN" -Atc "SELECT 1 FROM drugref.substance_moiety LIMIT 1"                  # permission denied: and nothing else
```

The grants are as follows:
- **Restored this way**, the owner owns the schema, so no grant is needed.
- **A non-superuser door owner, with drugref owned by another role**, needs USAGE on the schema `drugref`, EXECUTE on
  `drugref.allergy_check(uuid, uuid)`, and SELECT on **every** drugref table and view that `allergy_check` reads.
  That is not only the three that `cairn_drug_available()` checks: `moiety_allergy_fold`, `allergy_group_member`,
  `allergy_mesh_placement`, `identity_claim` and `substance_composition` too. Without them, the facts door raises and
  every line reads "not checked" (ADR-0098 decision 12). That is honest, but it is not a check.
- **Never grant drugref to `cairn_window`.** The four doors are its whole interface.

The drug codes the seed below needs come from the copy (`dr` is a function, like `cnode`):

```bash
dr() { psql "$CONN" -Atc "SELECT moiety_uuid FROM drugref.substance_moiety WHERE display_name = '$1'"; }
cmed() { cnode medication-assert "$1" --term "$2" --dose-amount 1 --dose-unit g --formulation tablet \
           --coding-system drugref-moiety --coding-code "$(dr "$2")" --coding-display "$2"; }
umed() { cnode medication-assert "$1" --term "$2" --dose-amount 1 --dose-unit g --formulation tablet; }   # uncoded
for d in amoxicillin flucloxacillin cephalexin metformin perindopril codeine; do [ -n "$(dr $d)" ] || echo "no moiety $d" >&2; done
```

**Setup B: the charts.** Use section 27's setup: sections 0 and 2, section 16's Setup and Measurement 2, `nurse-a`,
and the `cnode` and `achart` functions. Then:

```bash
K1=$(achart "Check Group" 1991-01-01)      # a GROUP allergy against five lines
K2=$(achart "Check Moiety" 1992-02-02)     # a DRUG allergy (amoxicillin) and an adverse effect
K3=$(achart "Check Typed" 1993-03-03)      # a typed (uncoded) allergy and a drug in no group
T1=$(achart "Check Timed" 1994-04-04)      # Measurement 3's entries
for c in "$K1" "$K2" "$K3" "$T1"; do [ -n "$c" ] || echo "a registration failed — read the output above" >&2; done
for c in "$K1" "$K2" "$K3"; do cmed "$c" amoxicillin; cmed "$c" flucloxacillin; cmed "$c" cephalexin; cmed "$c" metformin; done
umed "$K1" Augmentin; cmed "$K2" perindopril; cmed "$K3" perindopril
```

Launch as section 3 on `$K1` with `--key-hold-seconds 900`, and unlock the terminal.

**Measurement 1: the pick and the words, through the real `med_list` command** (ADR-0098 (d): no test drives the
whole command against a real drugref; this measurement does).

- [ ] **The pick.** On `$K1`, open *Show all allergies* and press *Drug*, then type `pe`: no list. Type `n`: within
      a blink, a list. With the real drugref, `pen`, `penic`, `penicillin` and `penicillins` each list the group
      **first**, reading "Penicillins (allergy group)" with no second "(group)", then drugs (ruling F2, #1057).
- [ ] Choose the group with ↓ and Enter. The field reads "Penicillins (allergy group)" and the list closes. Under the
      field, with no hover, a line reads **"Coded as: Penicillins (allergy group)"** (ruling F3). Press
      *Allergy*, *Anaphylaxis*, *Seen documented*, *Sign*. The strip reads "Allergy: Penicillins (allergy group) →
      anaphylaxis" with no suffix. The expanded list line ends **" (drug group not clinically reviewed)"**, because
      the group is `structural-seed`.
- [ ] **The medication list, redrawn without a refresh** (the act re-reads it). Each line reads as follows, and every
      tag and every row of words shows with no click or hover:
      - **amoxicillin**: the **Allergy** tag, and under the line "amoxicillin is a Penicillins (allergy group); allergy
        recorded: anaphylaxis";
      - **flucloxacillin**: the same, with "flucloxacillin is a …";
      - **cephalexin**: **no tag**. drugref answers `no-match`, because it has no cross-allergy rule (ADR-0098 (a),
        #1055). This is the known gap: note it, and do not file it again;
      - **metformin**: no tag (`no-match`, a verified negative);
      - **Augmentin** (uncoded): the small **not checked** tag and no words.
- [ ] The wire, at the console: `psql "$CONN" -c "SELECT term, coding FROM clinical_allergy WHERE patient_id = '$K1'"`
      shows `{"code": "55a1d1a0-…", "system": "drugref-class", "display": "Penicillins (allergy group)"}`. Check the
      class uuid against `psql "$CONN" -Atc "SELECT class_uuid FROM drugref.allergy_group"`.
- [ ] **A drug allergen.** On `$K2`, type `amox`. The list starts "amoxapine", "amoxecaine", then "amoxicillin": note
      whether a look-alike first option makes the wrong pick easy (amoxapine is an antidepressant). Press ↓ once and
      Enter: the line under the field reads **"Coded as: amoxapine (drug)"** — the wrong pick is visible. Type one
      letter: the line goes. Clear the field, type `amox` again and choose "amoxicillin": the line reads "Coded as:
      amoxicillin (drug)". Then *Allergy*, *Anaphylaxis*, *Sign*; the line goes with the emptied form. Then
      record an adverse effect: type `metf`, choose "metformin", *Adverse effect*, *GI upset*, *Sign*. The list reads:
      - **amoxicillin**: **Allergy**, "Allergy recorded: amoxicillin → anaphylaxis";
      - **flucloxacillin**: **Allergy**, "flucloxacillin is in the same group as amoxicillin (Penicillins (allergy
        group)); allergy recorded: anaphylaxis";
      - **metformin**: no tag, and a muted "Previous adverse effect: metformin → GI upset" (never the loud tag);
      - **cephalexin** and **perindopril**: no tag (`no-match`).
- [ ] **Not checked.** On `$K3`, type `penicillin` and *do not choose*: *Allergy*, *Rash*, *Sign*. Every coded line
      reads **not checked** (an uncoded allergen cannot be checked by code). Then type `codei`, choose "codeine",
      *Allergy*, *Rash*, *Sign*. drugref places codeine in no group (`allergen-in-no-group`), so every line still
      reads **not checked**, and none is marked (ruling R1).
- [ ] **An edit drops the coding.** On a fresh chart (`achart`), type `amoxi`, choose "amoxicillin", then add a
      letter to the field and delete it (the name reads as before), then sign. Separately, choose a drug and then
      press *Other substance*, then *Drug* again, then sign. In both cases the console row's `coding` is `null`.
- [ ] **Correct keeps the coding.** On `$K2`, press *Correct* on the amoxicillin line and change the reaction only,
      then *Sign*: the new row's `coding` is the same as the old one's. Press *Correct* again, change the *Substance*
      text, and *Sign*: `coding` is `null`.
- [ ] **No drug list.** At the console, run `psql "$CONN" -c "ALTER SCHEMA drugref RENAME TO drugref_off"`, then
      reopen `$K2`.
      - The pick shows "No drug list on this terminal: the name is recorded as typed" under *Substance*.
      - The amoxicillin line is still marked (by name and code). flucloxacillin, cephalexin, perindopril and
        metformin read **not checked**: there is no group answer without drugref, and the amoxicillin allergy can no
        longer be verified against them. metformin keeps its muted note.
      - Nothing fails, and nothing reads "could not be read".

      Then run `psql "$CONN" -c "ALTER SCHEMA drugref_off RENAME TO drugref"`.

**Measurement 2: the pane's open time, drugref present vs absent (+ ≤ 100 ms).** The window times no pane open, so
measure what this slice adds directly: the facts door, timed as the window, for five allergens by twenty lines (a
worst case worth timing). The window cannot read drugref, so build the two arrays as the owner:

```bash
A=$(psql "$CONN" -Atc "SELECT array_agg(moiety_uuid)::text FROM (SELECT moiety_uuid FROM drugref.substance_moiety ORDER BY moiety_uuid LIMIT 5) s")
M=$(psql "$CONN" -Atc "SELECT array_agg(moiety_uuid)::text FROM (SELECT moiety_uuid FROM drugref.substance_moiety ORDER BY moiety_uuid OFFSET 100 LIMIT 20) s")
for i in 1 2 3 4 5; do psql "$WINDOW_CONN" -c '\timing on' -c "SELECT count(*) FROM cairn_drug_facts('$A'::uuid[], '$M'::uuid[])"; done
```

- [ ] Record the five times. 100 pairs took about 5 ms on the maintainer's Mac (`drugref.allergy_check` called
      directly, 2026-11-10). A figure near 100 ms is the finding.
- [ ] Then by eye, or with a screen recording stepped frame by frame: open `$K1` five times with drugref present and
      five times with it renamed away, as above. Note any visible difference in when the list draws.

**Measurement 3: the timings (five runs each, held key, then shared terminal).**

- [ ] **A coded entry (≤ 15 s).** On `$T1`, with the form open: start at the first keystroke in *Substance*, type
      `amoxi`, ↓, Enter, *Allergy*, *Anaphylaxis*, *Seen documented*, *Sign*; stop when "Recorded." shows and the
      list's tags are redrawn. On later runs, pick `penicillins` (the group), `codei`, `trimeth` and `flucl`.
- [ ] **A free-text entry with the pick present (≤ 10 s).** The same gesture, ignoring the list: type `penicillin`,
      *Allergy*, *Anaphylaxis*, *Seen documented*, *Sign*. The list must not get in the way. Note whether Enter, Tab
      or the open list ever cost a step.
- [ ] **The pick (≤ 300 ms).** With a screen recording, step from the frame where the third letter appears to the
      frame where the list draws, five times. This is 150 ms of deliberate delay plus the search.
- [ ] Then the write halves: `psql "$CONN" -c "SELECT * FROM ui_gesture_timing WHERE gesture_kind = 'allergy-record'"`.
- [ ] Relaunch with no `--key-hold-seconds` on fresh charts and repeat both entries.

**Measurement 4: the real Tauri IPC** (the walk stubbed `invoke`). The webview sends `drug_search`'s `text`, and
`allergy_record`'s `coding` (`{system, code, display}`) beside `entry`. A name Tauri cannot map fails the call with an
"invalid args" error.

- [ ] The pick lists options. It never reads "No drug list on this terminal" while drugref is installed: that would
      mean the call failed and was read as no list (#1058).
- [ ] A picked entry's console row carries its coding (Measurement 1). A medication list that fails shows its own
      error, never an empty list.

**Keyboard and screen reader, same pass as section 6** (VoiceOver on macOS, keyboard only):

- [ ] **The loud tag.** A marked line's row header is read with the drug's name **and the word "Allergy"**, and each
      mark's row under the line is read as text. Neither relies on colour: the tag is a word.
- [ ] **The small tag.** An unchecked line's row header is read with "not checked". It is told apart from a checked
      line by that word, not by colour or weight.
- [ ] A muted adverse-effect row is read, and it is distinguishable from a mark by its words ("Previous adverse
      effect: …").
- [ ] "Allergies could not be read: medications not checked against them" is read when it appears (it is a status,
      above the list). It cannot be produced by hand on this rig; `allergy_check_db_tests.rs`'s
      `med_list_draws_when_the_allergy_read_fails` pins its words. If a real failure shows it, note whether it was read.
- [ ] *Substance* is announced as a combo box. Whether the option count, or the active option during ↓ / ↑, is
      announced (the list is a `listbox` with `aria-expanded` on the field). Escape closes it.
- [ ] The hint "No drug list on this terminal: …" is read or visible.
- [ ] No dialog at any point.

**WebKit.** The window is WebKit (Tauri on macOS). The webview was walked headlessly in Chromium, with a stubbed
`invoke` and rows drawn by `nameCell`/`allergyRows` directly; a headless WebKit walk covered only the pick's labels
and the "Coded as" line. **The real window's WebKit has not been walked.** Walk the pick in the real window and note anything that differs from the words above, in particular:
- ↓ / ↑ / Enter / Escape in the field;
- a click on an option, which moves the focus;
- the list left open when Tab leaves the field (there is no blur close, #1058);
- the tags' layout inside the row header on a narrow window.

**What could not be run here.** Every stopwatch figure, the VoiceOver pass, the real Tauri IPC, the real window's
WebKit and the full `med_list` command against a real drugref need the window and a human. The rest was run as
follows:
- **The floor:** `db/tests/074_clinical_allergy_test.sql` and `db/tests/076_drug_read_door_test.sql` section 4.
- **The doors against a stub:** `db/tests/076_drug_read_door_test.sql` and `drug_read_door.rs`,
  `drug_read_door_shape.rs` and `drug_answers_read.rs`. That includes `the_window_role_can_call_the_doors`.
- **The check:** `allergy/check_tests.rs` and `check_unverified_tests.rs`.
- **The window's commands:** `cairn-gui-tauri`'s DB-gated `allergy_check_db_tests.rs`, on the stub and on a
  rolled-back transaction (CI never runs those, #905).
- **The webview:** source guards (`drug_pick_webview_tests.rs`) and the stubbed walks (Chromium; the pick's labels and
  "Coded as" line in Chromium and WebKit).

The real drugref's group-first order (#1057) was checked only by running the door's query read-only against
`drugref_allergy`, never in the window. `TEMPLATE.md` carries a matching block (*The medication check*).

## 30. Charting a medication from one line, and the check at charting (charting slice 1a, ADR-0099)

The slice's gestures:
- **The presses.** Over a chart on screen the medication pane shows *Start medication* and *Already taking*. Either one
  opens one line, and the pressed one reads as pressed.
- **The line.** Type the drug, then the directions: `amox 500mg tds`. From the 4th letter of a drug part a list of
  drugs drops down (drugs only, never a group), the top one highlighted. Enter takes it, and "Coded as: …" shows under
  the line. A combination is typed as its parts, `amox + clav 875/125 bd`, one pick per part. Under the line the
  read-back row says "Drug · Form · Strength · Dose · Route · How often", "—" for each field not read, then "not read:
  …".
- **The check.** The line carries ADR-0098's **Allergy** tag and words, a muted note, or the small **not checked** tag,
  before Sign. On *Start medication* a mark also shows "Reason for charting despite the listed allergy", and Sign stays
  unavailable until something is typed there. *Already taking* shows the mark and asks no reason.
- **Cancel**, or Escape with no list open, discards the unsigned line.
- **After Sign** the line closes, "Charted." is said, and the list shows the line with its dose cell as `dose — sig`.

Paper counterpart: write "Amoxycillin 500 mg tds PO" on the drug chart and sign it; write "penicillin allergy —
benefit outweighs risk" beside a new order. **Live only**: a `--mock` window shows no presses.

**Budgets (the slice plan's §1.2).**

| Gesture | Start | Stop | Budget |
|---|---|---|---|
| Chart a common drug from the line, the pick present | the press of *Start medication* | "Charted." and the line drawn in the list | **≤ 10 s** |
| The same, no drug list (drugref renamed away) | the press | "Charted." | — (record) |
| Reading the line before Sign | the line typed | the drug, dose, route and any allergy hit read | no click, no hover |
| An override: Start against a listed allergy | the press | "Charted." | — (record; the reason typed is the one extra step) |
| A combination by its parts (`amox + clav 875/125 bd`) | the press | "Charted." | — (record; two picks) |
| *Already taking* × 10 lines (a home list) | the first press | the tenth "Charted." | — (record; #1072) |

The budgets are the plan's gestures, **with the key held** (`--key-hold-seconds 900`). Time them again on a shared
terminal (no hold, the clock starting at the press) and record both. Steps:
- chart a line: paper *N* = 1 (write and sign) → forced *M* = 1 (one signed event) → target *K* = 1 (the line and Sign);
- an override: *N* = 1 written note → *M* = 0 extra (it is in the event) → *K* = 1 typed field;
- *Start* vs *Already taking*: *N* = 0 on paper → *M* = 0 extra → the press replaces one Add;
- a home list of *n* lines: paper *N* = 1 signature → as built *n* Signs. **This is the `M > N` candidate filed as
  #1072**: record the figure there, do not re-file it.

The node times the write half of a charted line as **`medication-chart`** (section 5). **A figure outside its budget,
or an `M > N` other than #1072, is the finding: file an issue, and never adjust the budget.**

**Setup.** Section 29's Setup A (drugref with its allergy groups, owned by the owner) and its `dr` function. Build
this checkout first, let an owner `cnode` command replay the schema, and re-run section 0's `provision-window-role`,
because the build moves `SCHEMA_GENERATION` to 77. Then three charts with section 27's `achart`:

```bash
C1=$(achart "Chart Penicillin" 1981-01-01)   # a GROUP allergy: Penicillins (allergy group)
C2=$(achart "Chart Clean" 1982-02-02)        # no allergies
C3=$(achart "Chart Timed" 1983-03-03)        # the timings
for c in "$C1" "$C2" "$C3"; do [ -n "$c" ] || echo "a registration failed — read the output above" >&2; done
```

Launch as section 3 on `$C1` with `--key-hold-seconds 900`, and unlock the terminal. On `$C1`, record the allergy
in the window as section 29 Measurement 1 does: *Drug*, `penicillin`, choose "Penicillins (allergy group)",
*Allergy*, *Anaphylaxis*, *Seen documented*, *Sign*. On `$C2`, record "No known drug allergies".

**Measurement 1: the line, the pick and the read-back.**

- [ ] On `$C2`, press *Start medication*. It reads as pressed, and the field has the focus. Type `amo`: no list.
      Type `x`: a list of drugs, no group in it, each by its full name. Note the first option and whether it is the
      drug most charted on this node (on a fresh rig, the shortest name).
- [ ] Press ↓ until "amoxicillin" is highlighted, then Enter. "Coded as: amoxicillin" shows under the line. Type
      ` 500mg tds`. The read-back reads "Drug amoxicillin · Form — · Strength — · Dose 500 mg · Route — · How often
      three times daily". Sign.
- [ ] The list shows **amoxicillin** with the dose cell "500 mg — 500mg tds". At the console:
      `psql "$CONN" -c "SELECT charted_as, route, strength, components FROM medication_charting WHERE patient_id = '$C2'"`
      reads `started-here` with the rest empty.
- [ ] **The parse never guesses.** Without signing, type each of these and read the read-back, then Cancel:
      - `ceftriaxone 1g IV stat`: Dose 1 g, Route iv, How often "stat";
      - `amox 1,5mg tds`: Dose —, "not read: 1,5mg";
      - `amox 500mg 250mg tds`: Dose —, both doses under "not read";
      - `amox bd tds`: How often —, "not read: bd tds";
      - `amox tab 500mg`: Form tablet, Strength 500 mg, Dose — (no count is assumed).
- [ ] **An edit drops the coding.** Pick "amoxicillin", then change one letter of the drug word and change it back.
      "Coded as" is gone, and the line reads **not checked** until a new check answers. Sign: the console row of
      `medication_coding` has no row for that thread.
- [ ] **Cancel.** Open the line, type something, press Cancel: the line closes, nothing is signed, and the focus is
      on *Start medication*. Open it again and press Escape with no list open: the same. With the list open, Escape
      closes only the list.

**Measurement 2: the check at charting, and the reason.**

- [ ] On `$C1`, *Start medication*, `amox`, choose "amoxicillin", ` 500mg tds`. Before Sign, with no click: the
      **Allergy** tag and "amoxicillin is a Penicillins (allergy group); allergy recorded: anaphylaxis" on the line,
      and "Reason for charting despite the listed allergy" below it. Sign is unavailable.
- [ ] Type three spaces in the reason: Sign stays unavailable. Type `benefit outweighs risk`, Sign. "Charted.".
      `psql "$CONN" -c "SELECT override_reason, override_against FROM medication_charting WHERE patient_id = '$C1'"`
      reads the reason and one uuid. Check it against
      `psql "$CONN" -Atc "SELECT event_id FROM clinical_allergy WHERE patient_id = '$C1'"`.
- [ ] **Already taking.** On `$C1`, *Already taking*, `flucl`, choose "flucloxacillin", ` 500mg qid`. The mark shows,
      and no reason field. Sign. The row reads `pre-existing`, no override.
- [ ] **A combination.** On `$C1`, *Start medication*, type `amox`, choose it, then ` + clav`, choose "clavulanic
      acid" (or leave it unpicked if the drug list has none), then ` 875/125 bd`. "Coded as" reads both parts. The
      mark shows (through the amoxicillin part). Type a reason, Sign. The list's name reads "amoxicillin + clavulanic
      acid", and its line carries the **Allergy** tag. The console row's `components` holds both parts.
- [ ] **A brand typed as the drug.** On `$C1`, *Start medication*, `Augmentin 875/125 bd`, no pick (the drug list has
      no brand names, #1065). The line reads **not checked** and asks no reason. Note it: this is the gap #1065 owes.
- [ ] **The check is re-run at Sign.** An allergy recorded elsewhere while a line is open cannot be produced by hand
      on this rig: there is no console verb for an allergy, and an allergy recorded in this window re-reads the list,
      which checks the open line again. `med_chart_db_tests.rs::med_chart_db_the_check_is_reread_at_sign` pins it.
      What can be seen, on a fresh `achart` (not `$C1`, which Measurement 3 needs) with the Penicillins allergy
      recorded: open a *Start* line for amoxicillin, see its mark, then sign the allergy's *Entered in error* in the
      allergy list. The open line reads **not checked** at once, then clean, and the reason field goes.
- [ ] **No drug list.** `psql "$CONN" -c "ALTER SCHEMA drugref RENAME TO drugref_off"`, then on `$C1` press *Start
      medication* and type `amox`: "No drug list on this terminal: the medication is charted as typed". Type
      ` 500mg tds`. The line reads **not checked** and asks no reason (ADR-0099 (h): an uncoded line cannot be checked
      against a coded group allergy). Sign: it charts uncoded, and the list line reads **not checked**. Then
      `psql "$CONN" -c "ALTER SCHEMA drugref_off RENAME TO drugref"`.

**Measurement 3: the timings (five runs each, held key, then shared terminal).**

- [ ] **A common drug (≤ 10 s).** On `$C3`, from the press of *Start medication* to "Charted.": `amox`, Enter on the
      right option, ` 500mg tds`, Sign. On later runs: `paracet` ` 1g qid po`, `cefal` ` 500mg qid`, `metfo` ` 500mg
      bd`, `atorv` ` 40mg nocte`. Note how often the top option was the wrong drug.
- [ ] **No drug list.** The same gesture with drugref renamed away (record).
- [ ] **An override.** On `$C1`: the same gesture plus the reason (record).
- [ ] **A home list.** On a fresh `achart`, ten lines by *Already taking*, one after another (record; #1072).
- [ ] Then the write halves:
      `psql "$CONN" -c "SELECT * FROM ui_gesture_timing WHERE gesture_kind = 'medication-chart'"`.
- [ ] Relaunch with no `--key-hold-seconds` on fresh charts and repeat the common drug.

**Measurement 4: the real Tauri IPC** (the webview tests stubbed `invoke`). The webview sends `med_line_read`'s
`text`, `med_drug_search`'s `text`, `med_line_check`'s `patientId`, `charts`, `typedDrug` and `parts`, and
`med_chart`'s `patientId`, `charts`, `line` (`parts`, `typed_drug`, `directions`, `charted_as`, `reason`) and
`credential`. A name Tauri cannot map fails the call with an "invalid args" error.

- [ ] The read-back fills as you type, the pick lists options with drugref installed (never the no-drug-list hint),
      and the line's tag follows the allergies (Measurement 2).
- [ ] A signed line's console rows match Measurements 1 and 2.

**Keyboard and screen reader, same pass as section 6** (VoiceOver on macOS, keyboard only):

- [ ] The two presses are reached by Tab and read with their pressed state.
- [ ] The line is announced as a combo box. Note whether the active option is announced during ↓ / ↑ (it is not
      expected to be: #1070).
- [ ] Note whether the **Allergy** tag and its words are announced when they appear while typing (they are not live
      regions: #1070). Whether "Coded as: …" and the read-back are read (both are polite live regions).
- [ ] The reason field is reached by Tab and read with its label. A refusal under the line is read when it appears.
- [ ] A charted line's row is read with its dose cell, "500 mg — 500mg tds" (R26), never "dose not recorded" for a
      line with directions.
- [ ] No dialog at any point.

**WebKit.** The window is WebKit (Tauri on macOS). The webview was pinned by source guards and walked headlessly
with a stubbed `invoke`. **The real window's WebKit has not been walked.** Walk the line in the real window and note
anything that differs from the words above, in particular ↓ / ↑ / Enter / Escape in the field, a click on an option,
and the line on a narrow window.

**What could not be run here.** Every stopwatch figure, the VoiceOver pass, the real Tauri IPC and the real window's
WebKit need the window and a human. The rest was run as follows:
- **The wire:** `cairn-event`'s `medication/charting.rs` tests.
- **The floor, projection and door:** `medication_charting_floor.rs`, `medication_charting_projection.rs`,
  `medication_drug_search_door.rs`, `medication_chart.rs` (DB-gated, against the drug stub).
- **The parser and the check:** `medication/line_tests.rs`, `allergy/check_combination_tests.rs`.
- **The window's commands:** `cairn-gui-tauri`'s `med_chart_tests.rs` and `med_chart_rows_tests.rs` (pure) and
  `med_chart_db_tests.rs` (DB-gated; CI never runs those, #905).
- **The webview:** source guards (`med_chart_webview_tests.rs`, `med_chart_line_webview_tests.rs`).

`TEMPLATE.md` carries a matching block (*Charting a medication*).
