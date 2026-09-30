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

## 10. Unlink one link: "Not the same person…" (R2b-2, [#681](https://github.com/cairn-ehr/cairn-ehr/issues/681) · [#699](https://github.com/cairn-ehr/cairn-ehr/issues/699))

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

Start the stopwatch at the press of **"Not the same person…"** on a link's line, not before — choosing
WHICH link is wrong is the cognitive load and is inside the gesture only from the press; the panel then
lays the two charts side by side. Stop it when the outcome line reads "Unlinked — chart(s) … are no
longer part of this record" (or "Recorded that charts … are different people — but they still read as
one record through other links …" for a cycle (A–B, B–C, A–C: unlinking one edge leaves the other two joining the charts), a legitimate outcome that also stops the clock).

1. Press **"Not the same person…"** on the wrong link's line.
2. Read the panel: findings (if any), then the two-column comparison of the two charts.
3. Press **Unlink — not the same person**.

Budget **review-and-unlink ≤ 15 s** (§1.2 in the design page's R2b section). Record, per run: the wall
time from step 1 to the outcome line; the number of findings shown (0 is legitimate — nothing renders,
never "no conflicts"); whether the key was already unlocked. Repeat at least five times with fresh pairs.

**Accessibility, same pass as section 6** (VoiceOver on macOS, keyboard only):

- [ ] Each link's button is announced with its **own** text — which two charts, how the link was made
      (a clinician's judgement, or without one on record here), and when — never a bare
      "Not the same person…" repeated down the list.
- [ ] Pressing the button announces the findings **before** the table.
- [ ] An absent fact reads as a word, never silence.
- [ ] Opening the unlink panel closes the link panel and vice versa — the two are never on screen
      together.
- [ ] Every outcome (refusal, Outranked, StillJoined, a link that is gone) is shown AND announced.
- [ ] Close (Esc or "Close comparison") returns focus to the link button that opened the panel; after a
      successful unlink, focus lands on the patient's heading, not `<body>`.

Record in the template's *Unlink one link* section. **A figure outside the ≤ 15 s budget is a finding
to file, never a budget to adjust.**
