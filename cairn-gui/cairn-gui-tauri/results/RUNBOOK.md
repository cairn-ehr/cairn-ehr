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

A node database with `cairn_pgx` installed, and this node's key. For a throwaway rig — every
command below was run end-to-end on 2026-08-03, so the flags are the real ones:

```bash
export CONN="host=127.0.0.1 port=5532 user=$USER dbname=cairn_measure"
export NODE_KEY=/tmp/measure-node.key

psql -h 127.0.0.1 -p 5532 -d postgres -c "CREATE DATABASE cairn_measure"
psql "$CONN" -c "CREATE EXTENSION IF NOT EXISTS cairn_pgx"

cairn-node --conn "$CONN" --key "$NODE_KEY" \
    init --name measure-rig --address 127.0.0.1:0 --insecure-plaintext   # test rig only
```

> `--key` is a **global** flag: it comes before the subcommand, not after it. Every command
> below follows that shape.

## 1. Enrol the clinician who will sign

The window signs as an **enrolled human actor** (ADR-0053); an unenrolled key is refused at
unlock. Two clinicians need two *distinguishing determinants* — enrolling both as a bare
`{"role":"clinician"}` collides into one `actor_id` and is refused
([ADR-0044](../../../docs/spec/decisions/0044-enroll-fail-closed-on-actor-id-collision.md)).

```bash
cairn-node --conn "$CONN" --key /tmp/dr-a.key enroll-human --handle dr-a --insecure-plaintext
cairn-node --conn "$CONN" --key /tmp/dr-b.key enroll-human --handle dr-b --insecure-plaintext
```

> The window unlocks a **sealed** key with a passphrase. For a measured run that is the
> realistic path — seal `dr-a.key` (`cairn-node seal-key`) and use its passphrase at the
> unlock prompt. An unsealed key needs no passphrase and makes the unlock step vanish, which
> would understate the gesture.

## 2. Seed a five-drug chart, three of which are unsigned

The benchmark's row: *review a 5-drug list, sign 3 unsigned/stale lines.* So two lines must
already carry **someone else's** current signature — that is what proves the gesture leaves
another clinician's vouch alone rather than reassigning it.

Since [#345](https://github.com/cairn-ehr/cairn-ehr/issues/345) a chart must be **registered**
before anything can be recorded about it — the §5.3/§5.8 precedence rule, enforced in the
database. A hand-minted `uuidgen` id is refused by `medication-assert`, which is the point: you
cannot write on a chart nobody made, exactly as on paper. `patient-register` runs the
search-before-create funnel and prints the new id on its last line.

```bash
NODE="cairn-node --conn $CONN --key $NODE_KEY"
PATIENT=$($NODE patient-register --name "Bench Patient" --birth-date 1980-01-01 \
    --confirm-new | sed -n 's/^registered patient //p')
[ -n "$PATIENT" ] || { echo "registration failed — read the output above" >&2; exit 1; }

# Three lines nobody has signed.
$NODE medication-assert "$PATIENT" \
    --term atorvastatin --dose-amount 40 --dose-unit mg --formulation tablet
$NODE medication-assert "$PATIENT" \
    --term metformin --dose-amount 1 --dose-unit g --formulation tablet
$NODE medication-assert "$PATIENT" \
    --term sertraline --dose-amount 50 --dose-unit mg --formulation tablet

# Two lines Dr B has already signed — author-time attestation in one act.
$NODE medication-assert "$PATIENT" \
    --term amlodipine --dose-amount 5 --dose-unit mg --formulation tablet \
    --attest-as /tmp/dr-b.key
$NODE medication-assert "$PATIENT" \
    --term perindopril --dose-amount 4 --dose-unit mg --formulation tablet \
    --attest-as /tmp/dr-b.key

echo "PATIENT=$PATIENT"
```

Confirm the chart before measuring anything — if it does not read the way you expect, the
measurement measures the wrong thing:

```bash
$NODE medication-list "$PATIENT"
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
cargo run --release -p cairn-gui-tauri -- \
    --patient "$PATIENT" --conn "$CONN" \
    --key "$NODE_KEY" --attester-key /tmp/dr-a.key
```

`--release` matters: a debug build measures the compiler's tempo, not the design's.

## 4. The measured gestures

Start the stopwatch **when the window appears**, not when the command is typed — process
start-up is not a clinical act and is not what the budget is about.

1. **Review and sign.** Unlock the key, read the five lines, press *Sign off 3 unsigned
   medication(s)*. Stop the clock when the outcome line reports the result. Record the wall
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
    $NODE patient-register --name "$n" --birth-date "$dob" --confirm-new
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
cargo run --release -p cairn-gui-tauri -- --conn "$CONN" --key "$NODE_KEY"
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
$NODE patient-register --name "Bench Patient" --birth-date 1980-01-01 --confirm-new
cd cairn-gui
cargo run --release -p cairn-gui-tauri -- \
    --patient "$PATIENT" --conn "$CONN" \
    --key "$NODE_KEY" --attester-key /tmp/dr-a.key
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
- whether the clinician's key was already unlocked, or the run also measures an unlock.

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
      (compare again to get it back). A locked key is NOT such a refusal: the button stays —
      unlock, then press "Link — same person" again.
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

`$NODE` is section 2's (`cairn-node --conn $CONN --key $NODE_KEY`); `link-charts` takes only the
human's `--attester-key` (the top-level `--key` is already in `$NODE`). Register three charts, then
chain them A–B and B–C so that the wrong link (B–C) is the one that does not touch the chart you open:

```bash
reg() { $NODE patient-register --name "Bench Patient" --birth-date 1980-01-01 \
    --confirm-new | sed -n 's/^registered patient //p'; }
A=$(reg); B=$(reg); C=$(reg)
[ -n "$A" ] && [ -n "$B" ] && [ -n "$C" ] || { echo "registration failed" >&2; exit 1; }
$NODE link-charts "$A" "$B" --attester-key /tmp/dr-a.key
$NODE link-charts "$B" "$C" --attester-key /tmp/dr-a.key
cd cairn-gui
cargo run --release -p cairn-gui-tauri -- \
    --patient "$A" --conn "$CONN" \
    --key "$NODE_KEY" --attester-key /tmp/dr-a.key
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
> were reading, it is refused ("what stands between charts … changed while you were judging it … —
> nothing was done; review it again") and you compare again. An un-attested link also shows **Same
> person — confirm** beside it when both charts are held on this node; that verdict is timed in
> section 13, not here. If the pair's attestation changed after the list was drawn (a colleague
> confirmed it, say), the panel opens with no comparison and no verdict, only "this pair has been
> judged since the list was drawn — reload the chart…". A change that leaves the attestation as it
> was is caught later instead: each verdict is signed only over the judgement the panel read.

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
never "no conflicts"); whether the key was already unlocked. Repeat at least five times with fresh pairs.

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
reg() { $NODE patient-register --name "Bench Patient" --birth-date 1980-01-01 \
    --confirm-new | sed -n 's/^registered patient //p'; }
A=$(reg); B=$(reg)
[ -n "$A" ] && [ -n "$B" ] || { echo "registration failed" >&2; exit 1; }
uv run --project matcher --extra pipeline cairn-matcher watch --once --dsn "$CONN"
cd cairn-gui
cargo run --release -p cairn-gui-tauri -- \
    --patient "$A" --conn "$CONN" \
    --key "$NODE_KEY" --attester-key /tmp/dr-a.key
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
verdict; the number of findings shown (0 is legitimate); whether the key was already unlocked.
Repeat at least five times with fresh pairs.

**Measurement 2 — chart open.** Time opening chart A (stopwatch from launch or the open gesture, to
the medication table and banner painted) **with** the banner present, against the same node opening a
chart with **no** proposal (a single-chart open — e.g. a chart registered with nothing similar).
Budget: banner open **≤ single-chart open**. The banner reads eagerly (the other record's lines
included); a miss decides whether the other record's lines move to a second paint — it is a finding,
not an adjustment. Five opens each, same session state (key locked or unlocked, the same for both).

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
- [ ] **A chart that cannot be read** (#745 points 1–2): with a chart open, stop the node's Postgres and
      press **Sign off**. The outcome line says "Could not read the chart"; the banner does **not**
      vanish and offers no **Review** on a stale pair — it says "Could not check for possible
      duplicates: …". Restart Postgres, close the chart and reopen it: the banner is back as before.
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
        $NODE patient-register --name "$n" --birth-date "$dob" --confirm-new >/dev/null \
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
cargo run --release -p cairn-gui-tauri -- --conn "$CONN" --key "$NODE_KEY" --attester-key /tmp/dr-a.key
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
section 11's pair: one drain of the duplicate-check worker, then `cairn-node --conn <owner conn> --key
$NODE_KEY apply-auto-candidates` — an OWNER role, not `$CONN`'s (the command's own help says why). Its
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
reg() { $NODE patient-register --name "Bench Patient" --birth-date 1980-01-01 \
    --confirm-new | sed -n 's/^registered patient //p'; }
A=$(reg); B=$(reg); C=$(reg)
[ -n "$A" ] && [ -n "$B" ] && [ -n "$C" ] || { echo "registration failed" >&2; exit 1; }
$NODE link-charts "$A" "$B" --attester-key /tmp/dr-a.key
$NODE link-charts "$B" "$C" --attester-key /tmp/dr-a.key
$NODE link-charts "$A" "$C" --attester-key /tmp/dr-a.key
$NODE unlink-charts "$A" "$C" --attester-key /tmp/dr-a.key
cd cairn-gui
cargo run --release -p cairn-gui-tauri -- \
    --patient "$A" --conn "$CONN" \
    --key "$NODE_KEY" --attester-key /tmp/dr-a.key
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

**The locked key.** Run one pass with the key locked: both verdicts read "… (unlock your signing key
first)", and a press signs nothing (the panel's status line names the unlock; the button stays). Unlock (the existing session act, not one this
slice adds) and the suffix goes. Do not time the locked pass.

Budget **review-and-confirm ≤ 15 s** for both measurements (§1.2 in the design page's R6 section; the
same budget as section 10's unlink). Steps: confirm paper 2 → 2 acts; bridge paper 3 → 2 acts. Record,
per run: the wall time from step 1 to the outcome line; which measurement; the number of findings shown
(0 is legitimate on an undoubted link, never on Setup A); whether the key was already unlocked. Repeat
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
unsigned line on A, the window open on A with the key unlocked. A second node, or `cairn-node` on this same
node as another clinician, is ready to run `$NODE link-charts "$A" "$B" --attester-key <the other
clinician's key>` (`$NODE` as in section 13). Check the window shows no "How these charts are linked" line before you start.

**Measurement 1 — the refusal and the re-read.** Start the stopwatch at the press of **Sign off** (step 3);
stop it when the outcome line has finished saying "Now part of it: B.".

1. Open chart A.
2. From the second node, link A to B as the other clinician.
3. Press **Sign off** on A.
4. Read the outcome line: the refusal ("Sign-off failed: the linked charts changed while this list was on
   screen — nothing was done."), then "This record changed while it was on screen.", then "The chart has been
   read again — review it before you act on it.", then "Now part of it: <B's identity line>." The list now shows B's lines and the link line.
5. Press **Sign off** again; it succeeds.

**Measurement 2 — Read this chart again.** (a) Stop the database and press Sign off. Expect "Could not read
the chart"; the rows and the Sign off button stay. Restore the database and press **Read this chart again**;
time it from the press to the list drawn (1 act). (b) The "This list is out of date …" notice with Sign off
disabled fires only when a stale refusal succeeded and the re-read right after it failed, a one-round-trip
window: it is observable only with fault injection or the headless mock walk, never by hand. Do not look
for it in this pass.

**Check 3 — your OWN link is never announced.** Press **Same person as…** on A, compare with B, and link
them yourself. The outcome line reads "Linked — …" and nothing says "Now part of it". Then repeat with a
colleague's link landing at the same moment if you can; only the colleague's change may be named.

Budget: **re-read and sentence ≤ the single-chart open time (section 8 / section 11) after the press**;
**Read this chart again ≤ that same open time, 1 act**. Steps: paper N = 3 (write the chart, be told by a colleague or nurse, reconcile; commonly
the discrepancy is never noticed at all) → forced M = 3 (the refused press, review the re-read list, press
again) → bundling target K = 3. Failed read: today 3 acts (close, find, reopen) → 1. Record, per run: the wall time from step 3 to the end of the sentence; whether the sentence named
exactly the colleague's chart. Repeat at least five times.
**A figure above the budget, a sentence naming your own link, a sentence before the list drew, or a list that
vanished when the read failed is a finding to file, never a budget to adjust.**

**Accessibility, same pass as section 6** (VoiceOver on macOS, keyboard only):

- [ ] The refusal, "The chart has been read again …" and "Now part of it: B." are announced in order.
- [ ] "Read this chart again" is reachable by Tab at all times (also with no failure), and announced by that label.
- [ ] Focus stays where the clinician put it; nothing takes focus on its own.

`TEMPLATE.md` carries a matching block (*A colleague's link lands mid-review*).
