# Paper-parity measurement — YYYY-MM-DD, \<host>

> Copy to `YYYY-MM-DD-<host>.md`. Follow [`RUNBOOK.md`](RUNBOOK.md). Record what you
> measured, not what the budget hoped for.

## Rig

| | |
|---|---|
| Host / CPU / RAM | |
| OS + webview version | |
| Build profile | `--release` / `--debug` |
| PostgreSQL version | |
| Database size (`event_log` rows) | |
| Operator | |

## Chart under test

| | |
|---|---|
| Lines displayed | |
| Of which unsigned or stale | |
| Of which already signed by someone else | |
| Reconciled groups (rows with >1 thread) | |
| Groups missing from chart | should be 0 — see runbook §2 |

## Measured — whole gesture, by stopwatch

Excludes finding the patient (`--patient` at launch). Finding it is timed separately, in *Front door* below.

| Gesture | n | median | p95 | Provisional budget | Inside? |
|---|---|---|---|---|---|
| Chart open → list rendered → unsigned lines signed | | | | ≤ 15 s | |
| Cease one drug | | | | ≤ 5 s | |

## Measured — write cost only, from `ui_gesture_timing`

| gesture_kind | size_bucket | n | p50_ms | p95_ms |
|---|---|---|---|---|
| | | | | |

## Step count against the paper counterpart (§1.2)

Paper counterpart: the inpatient drug chart.

| Act | Paper *N* | Architecture-forced *M* | Observed UI *K* |
|---|---|---|---|
| Review a 5-drug list, sign 3 unsigned/stale lines | 3 signatures | 1 | |
| Cease one drug | 2 (strike + initial/date) | 1 | |

`M > N` would be an architecture defect and must be filed (#217 rule). Record `K` as
observed, including typing the staff ID and passphrase if no key was held.

## Accessibility pass

Screen reader + version: \_\_\_\_ · Keyboard only: yes / no

| Check | Verdict | Notes |
|---|---|---|
| Line announces drug, dose, status and signatory in one utterance | | |
| Chart warnings announced before the table | | |
| Stop button names its drug | | |
| Reason field names its drug | | |
| Sign-off button announces the real thread count | | |
| Every control Tab-reachable, focus ring visible | | |
| "Will be signed" identifiable without colour | | |
| "Ceased" identifiable without colour | | |

## Front door — find or register (§5.3/§5.8, runbook §8)

| Gesture | Mode | n | median | p95 | Budget | Inside? | Observed K |
|---|---|---|---|---|---|---|---|
| Find an existing chart (fragment → pick → header shows the name) | live | | | | ≤ 5 s | | |
| Find an existing chart | `--mock` | | | | ≤ 5 s | | |
| Register a new patient (first keystroke → new chart open) | live | | | | ≤ 20 s | | |
| Register a new patient | `--mock` | | | | ≤ 20 s | | |

Register someone already on file (`John Smith` / `1975-02-02`): was that chart **first** in the
prompt? **yes / no**

| Front-door accessibility check | Verdict | Notes |
|---|---|---|
| A candidate row announces name, age and identity state in one utterance | | |
| A prompt row's text includes "This is them" | | |
| Opening a chart moves focus to the patient's name | | |
| A failed search is announced as a failure, never as "no match" | | |

## Compare and link — "Same person as…" (R2b-1, runbook §9)

| Gesture | Mode | n | median | p95 | Budget | Inside? |
|---|---|---|---|---|---|---|
| "Same person as…" → Compare → read → Link → outcome line | live | | | | ≤ 20 s | |
| "Same person as…" → Compare → read → Link → fixture-refusal line | `--mock` | | | | ≤ 20 s | |

| Run | Findings shown | Key already held? |
|---|---|---|
| 1 | | |
| 2 | | |
| 3 | | |
| 4 | | |
| 5 | | |

| Compare-and-link accessibility check | Verdict | Notes |
|---|---|---|
| On Compare, veto findings announced before the table | | |
| Column groups announced as "This record" / "Other record" | | |
| An absent fact reads as a word, never silence | | |
| Link button Tab-reachable and announces disabled state (in flight); hidden, not disabled, for an incomplete comparison | | |
| Every panel outcome (refusal, fixture refusal, failed Compare, Outranked) shown AND announced | | |
| Close returns focus to "Same person as…"; a chart switch closes the panel without moving focus | | |

## Unlink one link — "Review this link…" (R2b-2, runbook §10)

| Gesture | Mode | n | median | p95 | Budget | Inside? |
|---|---|---|---|---|---|---|
| "Review this link…" → read → Unlink → outcome line | live | | | | ≤ 15 s | |

| Run | Findings shown | Key already held? |
|---|---|---|
| 1 | | |
| 2 | | |
| 3 | | |
| 4 | | |
| 5 | | |

| Unlink accessibility check | Verdict | Notes |
|---|---|---|
| Each link's button announced with its own text | | |
| Findings announced before the table | | |
| An absent fact reads as a word, never silence | | |
| The link and unlink panels are never open together | | |
| Every outcome (refusal, Outranked, StillJoined, link gone) shown AND announced | | |
| Close returns focus to the opening link's button; after a successful unlink focus is on the heading | | |

## Review a possible duplicate — the banner (R5a, runbook §11)

| Gesture | Mode | n | median | p95 | Budget | Inside? |
|---|---|---|---|---|---|---|
| Banner **Review** → read → Different people / Link → outcome line | live | | | | ≤ 20 s | |
| Chart open WITH a banner, vs a single-chart open (no proposal) | live | | | | banner open ≤ single-chart open | |

| Run | Verdict (Different people / Link) | Findings shown | Key already held? |
|---|---|---|---|
| 1 | | | |
| 2 | | | |
| 3 | | | |
| 4 | | | |
| 5 | | | |

| Banner accessibility check | Verdict | Notes |
|---|---|---|
| Banner announced as a region, never an alert; no re-announcement | | |
| Focus does not jump on chart open | | |
| Each Review announces whose record it opens | | |
| Heading list names each entry by its record, never N identical headings (#737) | | |
| Other record's medications announced under their own heading; a failed read is spoken | | |
| Review shows both verdict buttons only for a complete comparison; a refusal that cannot change hides both | | |
| Every outcome shown AND announced (Recorded/Linked on the chart line, refusals in the panel) | | |
| Close returns focus to the Review that opened it | | |

## Clear the possible-duplicate tray (R5b, runbook §12)

Entries in the tray at the start: \_\_\_\_ · status line shown above it? \_\_\_\_

| Gesture | Mode | n | median | p95 | Budget | Inside? |
|---|---|---|---|---|---|---|
| Tray CLOSED → expand → Review → banner Review → read → verdict → outcome line (5 acts) | live | | | | ≤ 25 s | |
| Tray already OPEN → Review → banner Review → read → verdict → outcome line (4 acts) | live | | | | ≤ 25 s | |
| Find (`mich`), no open proposals (tray hidden) | live | | | | ≤ 5 s | |
| Find, tray present and closed | live | | | | ≤ 5 s | |
| Find, tray OPEN, right after "Find another patient" | live | | | | ≤ 5 s, not slower than hidden | |
| Expand → Review → fixture chart open | `--mock` | | | | — (reported apart) | |

| Run | Tray open before? | Verdict (Different people / Link) | Entry gone and count one lower on return? |
|---|---|---|---|
| 1 | no | | |
| 2 | yes | | |
| 3 | yes | | |
| 4 | yes | | |
| 5 | yes | | |

| Tray accessibility check | Verdict | Notes |
|---|---|---|
| Summary announced with its count | | |
| Expands and collapses by keyboard alone | | |
| Each Review announces the newer record's first identity line | | |
| Heading list names each entry by both its records, never N identical headings (#796) | | |
| Nothing in the tray takes focus or is announced as an alert | | |
| On a non-Current node the status line is read before the tray | | |

## A colleague's link lands mid-review (#797, runbook §14)

| Gesture | Mode | n | median | p95 | Budget | Inside? |
|---|---|---|---|---|---|---|
| Sign off refused → chart re-read → "Now part of it: B." | live | | | | ≤ single-chart open time | |
| "Read this chart again" after a failed read (chart read paused by rename, then restored; 1 act) | live | | | | ≤ single-chart open time | |
| Press → failed-read line, the node not answering (§14 (d), table locked) | live | | | | ≤ 15 s (≤ 35 s queued) | |

| Run | Sentence named exactly the colleague's chart? | Own link announced? (must be no) |
|---|---|---|
| 1 | | |
| 2 | | |
| 3 | | |
| 4 | | |
| 5 | | |

| Accessibility check | Verdict | Notes |
|---|---|---|
| Refusal, "read again" and "Now part of it" announced in order | | |
| "Read this chart again" reachable by Tab at all times | | |
| Nothing takes focus on its own | | |
| Waiting bar: none on an ordinary press; announced at most twice ("Waiting…", "Reconnecting…") | | |

## Withhold and release (#832, runbook §15)

| Gesture | Mode | n | median | p95 | Budget | Inside? |
|---|---|---|---|---|---|---|
| Chart open → W recorded (until reviewed) | live | | | | ≤ 10 s | |
| Chart open → W recorded (until a time) | live | | | | ≤ 10 s | |
| Release | live | | | | ≤ 5 s | |

Steps: Withhold paper *N* = 2 → forced *M* = 2 → observed *K* = \_\_ · Release paper 1 → 1 → observed \_\_.
`M > N` is an architecture defect (file it).

| Behaviour | Verdict | Notes |
|---|---|---|
| W reads in local time | | |
| Empty or partial time refused, never "until reviewed"; select back to "until reviewed" empties the time | | |
| Second press during the write does nothing; fields cleared after success, kept (button live) after a refusal | | |
| A W starting or ending redraws the line by itself, within seconds | | |
| Line still counted by Sign off and still has Stop | | |
| Hover does not flicker | | |
| Withheld + will-sign line shows both bars | | |
| Release beside an undrawn colleague's W leaves it and says so | | |

| Accessibility check | Verdict | Notes |
|---|---|---|
| Tab reaches each W; text then reason read | | |
| Until select and time field labelled with the drug | | |
| Withhold and Release named by drug | | |
| W reason field names the drug | | |
| Ceased line's reason (when one was given) reads on focus | | |
| Pinned reason closes on Escape, and on a click elsewhere | | |
| Withheld line identifiable without colour | | |

## Sign in and sign (GUI sign-in slice 1, runbook §16)

Logging settings checked before the run (`log_statement`, `log_min_duration_statement`,
`log_parameter_max_length_on_error`, `pg_db_role_setting`): **safe / not safe** \_\_ ·
Window on the node's machine, or over an encrypted link: \_\_

| Gesture | Mode | Key hold | n | median | p95 | Budget | Inside? |
|---|---|---|---|---|---|---|---|
| Sign one act: first keystroke in Staff ID → outcome | live | 0 | | | | ≤ 3 s | |
| … of which press → outcome | live | 0 | | | | — | |
| Enrol one member: form opened → "is enrolled" | live | 0 | | | | (record) | |

Steps: sign paper *N* = 1 → forced *M* = 1 → observed *K* = \_\_ · next nurse paper 0 → Cairn 0 → observed extra
acts \_\_. `M > N` is an architecture defect (file it).

| Behaviour | Verdict | Notes |
|---|---|---|
| Lock: wrong / right / not set / node stopped each say their own sentence | | |
| Enrol: mismatch, duplicate, clinician credential each refused with their sentence; fields emptied | | |
| A superuser's key is never held, and is refused on a clinical write | | |
| A staff ID with capitals and spaces signs as the enrolled member | | |
| Hold 900: held key serves; the next nurse's credential replaces it; Lock clears it; a hold runs out | | |
| Back-off: tries 1–3 free, try 4 "Wait 1 s", a press inside the wait is "Too many tries" and not counted | | |
| Staff ID retyped after a refusal (count) | | |

| Accessibility check | Verdict | Notes |
|---|---|---|
| Lock screen, strip and enrol form complete by keyboard alone | | |
| Every field announced by its label | | |
| Each refusal and back-off sentence read once, whole; no passphrase spoken | | |
| "Signed in for the next act" announced; nothing takes focus on its own | | |

## Enrol in person, re-key, change a passphrase, confirm (GUI sign-in slice 2, runbook §17)

Site expiry during the timed runs: **off** \_\_ · Window on the node's machine: \_\_

| Gesture | n | median | p95 | Budget | Inside? |
|---|---|---|---|---|---|
| Enrolment by an enroller: "I'm new" → the newcomer's first own signature | | | | ≤ 2 min | |
| Re-key in person: form opened → first signature with the new passphrase | | | | ≤ 1 min | |
| Change my passphrase: form opened → "Your passphrase is changed." | | | | ≤ 30 s | |
| Superuser confirm: press → row leaves the worklist | | | | ≤ 10 s per row | |

Steps: enrolment paper *N* = 0 → forced *M* = 2 → observed *K* = \_\_ · re-key paper 0 → *M* = 2 → observed \_\_ ·
change of passphrase paper 0 → *M* = 1 → observed \_\_ · confirm 1 press per row → observed \_\_. `M > N` is already
filed (#919); record the counts against that issue.

| Behaviour | Verdict | Notes |
|---|---|---|
| Grant: "now has the enroller role"; a non-superuser's credential refused; strip passphrase emptied | | |
| Enroller enrols a clinician; refused for admin or superuser; newcomer's key on the worklist | | |
| Re-key: old passphrase answers "replaced on …, approved by …" and counts as a try; same actor (`rotate-key`) | | |
| Superuser re-keyed only at the console (`vault-rekey-superuser`) | | |
| Change of passphrase: works; wrong current counted; same-as-current refused | | |
| Confirm leaves the worklist; Repudiate asks no question and the member is then refused | | |
| Repudiate key on a confirmed member's row; the row then says repudiated; their unconfirmed approvals turn doubtful | | |
| Re-key success line: enroller's says a superuser must confirm; superuser's does not | | |
| Expiry: strip warns naming the ID; worklist shows the stop date; an expired key refused, then confirmed | | |
| Measurement 7, run LAST (empties the vault): a re-key without name/contact refused before any release; with them, re-keyed on the same actor, superuser wording | | |
| Window DB-gated tests run by hand (#905): no `skipped:`, the five named tests pass | | |

| Accessibility check | Verdict | Notes |
|---|---|---|
| Both new forms and the panel complete by keyboard alone; `admin` reachable in Enrol staff | | |
| Every new field announced by its label; row buttons announced with their row | | |
| Every outcome sentence read once, whole; no passphrase spoken | | |
| A double press on Confirm sends one act; nothing takes focus when the worklist redraws | | |

## Write, autosave and sign a progress note (GUI sign-in slice 3a, runbook §18)

Window on the node's machine: \_\_ · Terminal-id file present after first launch: \_\_

| Gesture | n | median | p95 | Budget | Inside? |
|---|---|---|---|---|---|
| Open the editor: *New note* → caret in *Progress note* | | | | ≤ 1 s | |
| Sign a note: first keystroke in Staff ID → "Signed — the note is on the chart." | | | | ≤ 3 s | |
| Sign a note: press → outcome share | | | | — | |
| Crash recovery: acts before typing can continue | | | | 0 | |

Steps: writing and signing paper *N* = 3 → forced *M* = 3 → observed *K* = \_\_ · reading the notes 0 → 0 → observed
\_\_ · after a crash paper — → *M* = 0 → observed \_\_. `note-sign` write cost (section 5): \_\_ ms.

| Behaviour | Verdict | Notes |
|---|---|---|
| Text that looks like markup signs and reads back exactly as typed; stored escaped | | |
| No stated time: "no stated time · writing began …", basis `recording-started`; a stated time: "(stated)" | | |
| `recording_ended` is the last keystroke, not the press | | |
| A partly typed time refuses Sign with its sentence; the saved stated time is kept | | |
| The editor and time field are read-only while a Sign is in flight | | |
| A stated time with seconds survives a reopen and signs with its seconds | | |
| A hard kill: the draft is back on relaunch; loss within a second of a keystroke: \_\_ | | |
| Node unreachable: "not saved yet — your text is still here"; Sign says it cannot sign; next keystroke saves | | |
| Another terminal's draft listed read-only; editable again with the terminal id restored | | |

| Accessibility check | Verdict | Notes |
|---|---|---|
| New note, editor, time field and Sign complete by keyboard alone; Enter in the time field signs nothing | | |
| Editor and time field announced by their labels | | |
| Each note read with its text, grade and time; the withheld sentence read | | |
| Sign sentence read once, or twice (`#note-status` + `#outcome`) — record which | | |
| No dialog; nothing takes focus on its own | | |

## Who's here, the idle re-lock and a draft's parts (GUI sign-in slice 3b-1, runbook §19)

Staff on the node: \_\_ (50 for Measurement 1) · `--idle-lock-minutes`: \_\_ · `--key-hold-seconds`: \_\_

| Gesture | n | median | p95 | Budget | Inside? |
|---|---|---|---|---|---|
| Hot key → "Writing as …", a recent person | | | | ≤ 3 s | |
| Hot key → "Writing as …", someone never picked here | | | | ≤ 3 s | |
| Re-lock: first passphrase keystroke → back at the chart | | | | ≤ 10 s | |
| Continue here: press → caret in *Progress note* | | | | ≤ 1 s | |
| A switch within a second of a keystroke: characters lost | | | | 0 | |
| Continue here with nobody named: acts / time to the caret (#974) | | | | paper 1 | |
| Reopen a draft after a hard kill: acts / time before typing (#974) | | | | 3a: 0 | |

Steps: writing and signing paper *N* = 3 → forced *M* = 3 → observed *K* = \_\_ · adding below a colleague 1 → 1 →
observed \_\_ · finishing at another terminal 1 → 1 → observed \_\_ · resuming after the re-lock 2 (+ walk) → 3 →
observed \_\_ · finishing elsewhere with nobody named 1 → 2 → observed \_\_ · after a crash (3a's target 0) observed \_\_.

| Behaviour | Verdict | Notes |
|---|---|---|
| The chart leaves the screen before the picker; no passphrase field; a held key is forgotten | | |
| Staff ID prefilled from the pick; passphrase empty; *Don't know* and the re-lock leave Staff ID empty | | |
| *Return to …* offered and focused, never auto-opened; what it shows after the name: \_\_ | | |
| Enter on an empty search names nobody; *Back* keeps the name | | |
| Nurse A's part holds every character after a mid-typing switch; Dr B's text never lands in it | | |
| A, B, A gives three parts; part 1 unchanged; one revision row per real change | | |
| Signed: part 1 `drafted` + `attested`, own part `authored`, later parts `addendum-to` the one before | | |
| Re-lock: chart gone, lock focused, bar "Nobody has said…", Staff ID empty | | |
| Typing in any field never re-locks; an untouched window with the poll running does | | |
| A Sign in flight is waited for by the re-lock; what the clinician is told: \_\_ | | |
| Moved: one new word kept as a new draft; nothing new closes the editor; an edited saved text keeps all | | |

| Accessibility check | Verdict | Notes |
|---|---|---|
| Hot key, picker and offer complete by keyboard alone; focus never left on a hidden element | | |
| The bar read once per change, never per poll | | |
| A colleague's part read "unverified Nurse A"; an unknown writer "writer not known" | | |
| A signed note's byline read "written by unverified Nurse A · signed by Dr B" (#973) | | |
| No dialog | | |

## A draft left untouched goes in unsigned (GUI sign-in slice 3b-2a, runbook §20)

Quarantine set: \_\_ minutes · `--idle-lock-minutes`: \_\_ · the window holds the node key: \_\_

| What | n | median | p95 | Budget | Inside? |
|---|---|---|---|---|---|
| The tick: `due` → the note in `clinical_note`, window on another chart | | | | ≤ 60 s | |
| A chart open that commits first | | | | single-chart open + 200 ms | |
| The same chart open with no draft (the baseline) | | | | — | |

Steps: walking away paper *N* = 0 → forced *M* = 0 → observed \_\_ · reading a note that went in unsigned 0 → 0 →
observed \_\_.

| Behaviour | Verdict | Notes |
|---|---|---|
| Before due: "goes into the record unsigned at HH:MM if nobody types in it", matching `due` | | |
| Two writers: two node-signed notes, `drafted` + `recorded`, the second `addendum-to` the first; `by_timer` | | |
| Read "written by ?Nurse A · went in unsigned"; never "signed by" | | |
| The open commits first; with no `--key` the chart reads and the draft says overdue | | |
| Typing into a committed draft: the committed sentence, a new draft; after editing the saved text, all kept | | |
| Sign on a committed draft: "had already gone into the record unsigned — nothing was signed" | | |
| Stuck: kept, nothing in, the stuck sentence; before due the no-time stuck sentence; stderr once | | |

| Accessibility check | Verdict | Notes |
|---|---|---|
| A went-in-unsigned note read "written by unverified Nurse A · went in unsigned" | | |
| Due, overdue and stuck words read with the draft | | |
| The committed sentence read once | | |
| No dialog | | |

## Throw away your own draft (GUI sign-in slice 3b-2b, runbook §21)

| Gesture | n | median | p95 | Budget | Inside? |
|---|---|---|---|---|---|
| Discard a draft: Staff ID keystroke → "Discarded. …" | | | | ≤ 3 s | |
| `ui_gesture_timing` `note-discard` (the write half) | | | | — | |

Steps: discarding your own draft paper *N* = 1 → forced *M* = 1 → observed \_\_ · a colleague's draft: refused, no
gesture · seeing a discard 0 → 0 → observed \_\_.

| Behaviour | Verdict | Notes |
|---|---|---|
| Own draft: no dialog; the draft gone; one event signed by the discarder, `authored` + `recorded`; `spent_at` = `t_effective` | | |
| Empty editor: "The draft was empty, so nothing was kept."; no event, no spend, no marker | | |
| No key held: "Not discarded: Enter your staff ID and passphrase to discard this draft." — never "sign" | | |
| A colleague's draft: the "someone else wrote part of this draft" sentence; draft kept; text saved again | | |
| A stuck draft: the same sentence; kept; its line reads the stuck words, no time | | |
| During the debounce: the audit copy ends with the last characters typed | | |
| A switch during a discard: the picker says the discard's outcome, never "signed" | | |
| WebKit (the real window) against the headless walk: any difference noted | | |

| Accessibility check | Verdict | Notes |
|---|---|---|
| *Discard draft* by keyboard; both buttons down while it runs | | |
| The marker read as a collapsed disclosure; opened by Space/Enter; parts read as typed | | |
| The outcome read once; a refusal's sentence in full | | |
| No dialog | | |

## Add to a note, and read a note with its parts (GUI sign-in slice 3c-1, runbook §22)

| Gesture | n | median | p95 | Budget | Inside? |
|---|---|---|---|---|---|
| Add and sign: Staff ID keystroke → the addendum inside the note | | | | ≤ 3 s | |
| `ui_gesture_timing` `note-sign` (the write half) | | | | — | |
| Open a compound's parts: the press → every part on screen | | | | ≤ 1 s | |

Steps: adding to a note paper *N* = 3 → forced *M* = 3 → observed \_\_ · who wrote which part, and when, 0 → 0 →
observed \_\_ ·
a linked chart's note 3 → 3 → observed \_\_ (#993).

| Behaviour | Verdict | Notes |
|---|---|---|
| Add opens the editor with "Adding to the note of …"; with nobody named, the picker asks first | | |
| The addendum signs inside the note: Nurse A's heading and "every part signed" ABOVE the text, a closed "2 parts"; `refs` names the note | | |
| The addendum's seam line, "Added: signed by Dr B · {its time}", sits before its text; the note keeps its place | | |
| Sign pressed inside the debounce: still an addendum, never a new note | | |
| A restart mid-addendum: the draft reopens with its line and signs inside the note | | |
| Two addenda signed at once on two terminals: both inside the one note, in order begun | | |
| A timer-committed addendum: "includes an unsigned part" with no press; its seam reads "Added: written by ?… · went in unsigned · …" | | |
| The orphan: heading "Adds to a note not shown here · …"; the draft begun before the `DELETE` reads "Adding to a note not shown here"; one note again after the reproject | | |
| WebKit (the real window) against the headless walk: any difference noted (a stale button: #994) | | |

| Accessibility check | Verdict | Notes |
|---|---|---|
| *Add to this note* by keyboard, read as "Add to this note: {time}"; none on a linked chart's note | | |
| "{n} parts" read as collapsed; opened by Space/Enter; each part's heading and text read | | |
| The "Adding to …" line read; the heading and state line read before the text | | |
| Each seam read before its part, with that part's writer and time; a claimed name heard "unverified …" | | |
| No dialog | | |

## Sign off a note, and sign your own earlier part later (GUI sign-in slice 3c-2, runbook §23)

| Gesture | n | median | p95 | Budget | Inside? |
|---|---|---|---|---|---|
| Sign off: Staff ID keystroke → "Signed off — …" and the byline re-read | | | | ≤ 3 s | |
| `ui_gesture_timing` `note-sign-off` (the write half) | | | | — | |
| Finish your own unsigned note: Staff ID keystroke → the earlier part reads "signed later by …" | | | | ≤ 3 s | |

Steps: countersigning paper *N* = 2 → forced *M* = 2 → observed \_\_ · finishing one's own unsigned note 3 → 3 →
observed \_\_ · seeing a student's part not yet countersigned 0 → 0 → observed \_\_.

| Behaviour | Verdict | Notes |
|---|---|---|
| A student's part reads "(student) — not co-signed" and offers *Sign off this note*; a note one doctor signed alone offers none | | |
| A doctor's sign-off: "· co-signed by Dr B", the mark cleared, the button gone; one `sign-off`/`co-signed` vouch row on the part's chart | | |
| Another student's co-sign leaves "— not co-signed"; a student on their own note is told "already signed every part" | | |
| A colleague's addendum lands after the note was shown: "This note changed since it was shown …"; nothing written | | |
| Nothing opens, and no other press starts, while a sign-off is in flight | | |
| The covers line names the writer's own unsigned part; Sign says it "now reads as signed"; the state line drops "includes an unsigned part" | | |
| A restart mid-addendum: the draft reopens with its covers line | | |
| Who's here A, strip B: "1 listed part(s) were not covered …"; A's part stays unsigned | | |
| "signed off by Dr B" on a note with a timer part | | |
| "(signer unconfirmed)" on an unconfirmed key's part, gone after *Confirm* | | |
| WebKit (the real window) against the headless walk: any difference noted (#994, #1001) | | |

| Accessibility check | Verdict | Notes |
|---|---|---|
| *Sign off this note* by keyboard, read as "Sign off this note: {time}" | | |
| "(student) — not co-signed", "(signer unconfirmed)" and each vouch read with the heading | | |
| The covers line read after "Adding to …" and before the text box | | |
| The outcome lines read when they appear | | |
| No dialog | | |

## A repudiated signature counts as unsigned (GUI sign-in slice 3c-3b, runbook §24)

| Gesture | n | median | p95 | Budget | Inside? |
|---|---|---|---|---|---|
| Re-sign one's own repudiated part: Staff ID keystroke → "Signed off — …" and the byline re-read | | | | ≤ 3 s | |
| `ui_gesture_timing` `note-sign-off` (the write half) | | | | — | |

Steps: seeing that a signature no longer counts paper *N* = 1 → forced *M* = 0 → observed \_\_ · re-signing 2 → 2 →
observed \_\_.

| Behaviour | Verdict | Notes |
|---|---|---|
| After *Repudiate key* and a re-key, each part reads "(key had been repudiated) — counts as unsigned"; the state line "includes an unsigned part"; *Sign off this note* offered | | |
| A colleague's sign-off: "· signed off by Dr B", `attested` (never `co-signed`), the part still "counts as unsigned", no "signed off by" state | | |
| The colleague's second press: "already signed every part" (#1008, known) | | |
| The real person is offered the press, not "already signed"; "· re-signed by Nurse A"; the mark and "includes an unsigned part" gone | | |
| Her own two-part note, nobody else's vouch on it: one press, each part "· re-signed by Nurse A", the state "signed off by Nurse A" | | |
| A note she and Dr B wrote: her press also vouches Dr B's part, "· co-signed by Nurse A", the state "signed off by Nurse A" (#1011, known) | | |
| A repudiated student part: "(student) (key had been repudiated) — counts as unsigned", no "— not co-signed"; after re-sign "— not co-signed · re-signed by Sam (student)"; a doctor's sign-off then "· co-signed by Dr B", the mark cleared | | |
| A doctor's sign-off BEFORE the student's re-sign: "— not co-signed" stays, no button (#1010, known) | | |
| WebKit (the real window): wrapping of the longer bylines | | |

| Accessibility check | Verdict | Notes |
|---|---|---|
| "(key had been repudiated) — counts as unsigned" and "· re-signed by …" read with the heading | | |
| "includes an unsigned part" read before the text | | |
| *Sign off this note* by keyboard, read as "Sign off this note: {time}" | | |
| No dialog | | |

## Verdict

- Observed p95 inside the provisional budget? **yes / no**
- If no: issue filed \_\_\_\_. **Do not adjust the budget to match the measurement.**
- Anything that surprised the operator:
