# ADR-0095 — The person menu

- **Status:** Accepted (ends [ADR-0092](0092-the-vouch-and-the-sign-off.md)'s "not guaranteed" (d),
  [ADR-0093](0093-a-repudiated-signature-counts-as-unsigned.md)'s (e) and
  [ADR-0094](0094-supervision-and-the-mandatory-co-sign.md)'s (n), as decision 16 says; supersedes no decision; builds
  the slice 3c-3 design's §3.4)
- **Date:** 2026-11-07
- **Spec version at acceptance:** 0.97
- **Issues:** draft PR [#1019](https://github.com/cairn-ehr/cairn-ehr/pull/1019) (GUI sign-in slice 3c-3c). Design:
  `docs/superpowers/specs/2026-11-06-sign-in-slice-3c-3-supervision-and-pending-signatures-design.md` (§3.4, §6);
  plan: `docs/superpowers/plans/2026-11-07-sign-in-slice-3c-3c-person-menu.md`. Touches
  [#963](https://github.com/cairn-ehr/cairn-ehr/issues/963) (not guaranteed (b)) and
  [#905](https://github.com/cairn-ehr/cairn-ehr/issues/905) ((i)). Residual:
  [#1020](https://github.com/cairn-ehr/cairn-ehr/issues/1020) ((j)) and
  [#1021](https://github.com/cairn-ehr/cairn-ehr/issues/1021) ((h)).
- **Builds on:** [ADR-0094](0094-supervision-and-the-mandatory-co-sign.md) (the supervision record, `awaits_cosign`,
  `reviewable`) · [ADR-0093](0093-a-repudiated-signature-counts-as-unsigned.md) (`counts_as_unsigned`) ·
  [ADR-0092](0092-the-vouch-and-the-sign-off.md) (`part_is_signed`, the vouch read) ·
  [ADR-0089](0089-the-quarantine-commit-and-the-asserted-grade.md) (a draft's due time, its "stuck" flag) ·
  [ADR-0088](0088-whos-here-and-a-drafts-parts.md) (who's here is a claim) ·
  [ADR-0084](0084-the-window-never-holds-the-owners-keys.md) (the window's own login) ·
  [ADR-0021](0021-layering-the-node-api-and-ui-pluralism.md) (logic below the UI)

## Context

On paper, a supervisor finds the entries waiting for their countersignature by walking the ward or a stack of
folders, and audits a supervisee by pulling files at random. A clinician finds their own unsigned entries the same
way. Until this slice Cairn was no better: a draft, a part that went in unsigned, a part signed with a since-repudiated
key, a supervisee's part awaiting a co-sign and a supervisee's note to review were each visible only on their own
chart. This slice, **3c-3c**, puts them in one place on the who's-here bar, "Waiting for you: N", for the person
who's here names. No event type, wire, floor rule or projection changes. db/073 adds two read-only doors
(`SCHEMA_GENERATION` 73); the groups are one pure module over the rows the chart already reads.

## Decision

### The maintainer's rulings (2026-11-07, plan review: Q1–Q3 as their defaults)

1. **The picker shows each person's count (Q1)**, from one read of every person's parts on each opening of the
   picker. The list draws first and the counts fill in when they land. The query carries a `shortcut:` comment: it
   grows with all-time history, not with what is open (not guaranteed (h)), and the projection of open obligations
   that would end that is [#1021](https://github.com/cairn-ehr/cairn-ehr/issues/1021). RUNBOOK §26 measures it. The
   alternative, moving the counts to a later slice, was not taken.
2. **The menu shares the chart list's query, only its `WHERE` changed (Q2).** That query reads
   `clinical_note.narrative`, the clear projection the chart list already reads; it opens no sealed body, and the
   menu shows none of the text. A second column list would be a second thing to keep in step.
3. **"Open note" from an open chart goes through `closeChart()` first (Q3)**, so queued saves land and panels close.
   The front door shows for a moment: the path "Who's here?" already takes.
4. **"Open note" is not an `M > N` defect** (design §6, 2026-11-07): paper's "open the folder" includes finding the
   folder and walking to it, and "Open note" does not.

### Whose menu, and what it counts

5. **The menu is the person a who's-here claim NAMES** (design §3.4, ADR-0088). Under "Not asked" or "Don't know"
   the bar gets no menu, no count and no database read, and its menu element is hidden. The picker's counts are read
   on each opening whatever the claim, so the picker shows them with nobody named. A named staff id the node cannot
   place reads as an empty menu. Reading the menu signs nothing and asks for no passphrase: like the chart, it is
   shown to whoever is at the terminal (design §2's anti-goals).
6. **N counts groups 1 and 2 only** (`PersonMenu::count`, `own + cosign`). The review group is never counted: nothing
   is owed there, and a number that grows with every supervisee's note would train people to ignore it (design
   §3.4; maintainer: it must not nag). The bar reads "Waiting for you: N", named "Waiting for you: N — open the
   list". The picker's suffix " · N waiting" (nothing for 0) is the same `PersonMenu::count`, over a read of every
   person's keys instead of the named person's, so while nothing changes between the two reads a person's count in
   the picker equals the bar's once they are named.

### The groups (`note::menu`, pure)

7. **Each group's predicate is the chart's own**, never a second reading of the same facts, so the menu and the
   byline cannot disagree about what is unsigned. A part is listed in at most one group, tried in the order 1, 2, 3.
   - **Group 1, "Your signature":** each open draft holding a non-blank part whose writer is the person's staff id;
     each part that went in unsigned (`signed_by` is `None` and `!part_is_signed`) whose `drafted_actor` is the
     person, so a colleague's later sign-off takes it off; each part that `counts_as_unsigned` whose `signed_by`
     actor is the person, so only their own re-sign takes it off (a repudiated key still belongs to the real
     person's actor, ADR-0085).
   - **Group 2, "Needs your co-signature":** each part that `cosign::awaits_cosign`, whose `supervision::writer_actor`
     is not the person, and whose `writer_supervisors` (the writer's supervisors NOW) include the person. The
     requirement is judged at the part's time, so lifting it later leaves a part signed under it listed and counted;
     a supervisor who has stepped away sees nothing of that supervisee; a writer with no supervisor now is in nobody's
     group 2. A part that counts as unsigned owes no co-sign (`cosign_required`), so it is in its real person's
     group 1 and in nobody's group 2. The row names the writer's other current supervisors.
   - **Group 3, "Your supervisees' recent notes (last 7 days)":** each part `review::reviewable` by the person
     (made `pub` for this) whose `recorded_at` is no older than `REVIEW_DAYS` before now.
8. **The review window is 7 days** (`REVIEW_DAYS`, design §3.4, *my call*; the maintainer did not object), judged on
   the part's `recorded_at` (its `hlc_wall`), never on its freely backdatable `t_effective`.
9. **Order:** within group 1 the drafts come first, by due time, soonest first; every other row is newest
   `recorded_at` first, `event_id` breaking a tie (the chart breaks ties the same way, but orders by `t_effective`).

### The read (`note::read`, `note::menu_read`, db/073)

10. **Two read-only SECURITY DEFINER doors, granted to `cairn_window` only and revoked from PUBLIC** (db/073). Each
    exists because the facts it answers sit in tables the window's login cannot read: `vault_member` is granted to no
    role (db/061), `actor_event` gives the window its `signing_key_id` column only (db/062), and `note_draft` /
    `note_draft_part` are revoked from PUBLIC (db/065, db/066).
    - **`cairn_note_people()`** turns keys into people: one row per distinct actor that is a vault member or that the
      supervision record names (as subject or supervisor), with its staff id (`cairn_supervision_staff_of`: the
      member row's, else the handle pinned on its enroll row), its display name (NULL without a member row, worded
      "a writer this node cannot name") and EVERY key it ever enrolled or rotated to, repudiated ones included, so a
      part signed with an old key maps back to its person.
    - **`cairn_note_drafts_by_writer()`** finds who has written into which open draft: one row per draft and distinct
      named writer of a non-blank part (the commit door's blank test). `due` and `stuck` are
      `cairn_note_drafts_due`'s (db/067) expressions, so the menu and the chart's draft row agree; the door reads no
      clock, so the window decides "overdue". No note text leaves either door.
11. **One query, two `WHERE`s.** The `notes_sql!` macro builds the chart's `NOTES_SQL` (by chart) and
    `NOTES_BY_KEYS_SQL` (the parts a set of keys signed, or carries as a contributor, which is where a part that went
    in unsigned names its drafter) from one column list, and `note_row` is the one parser of both, annotated against
    the supervision record. So every grade, vouch and supervision fact on a menu row is computed as the chart computes
    it. The doubted-link withholding of `list_patient_notes` is not applied: it guards combining linked charts on one
    opened chart, and the menu opens each part's own chart.
12. **One snapshot.** `read_menus` reads the people, the supervision record, the drafts and the parts on the caller's
    client, and the window calls it inside `in_snapshot` (REPEATABLE READ), with the row charts' names, so a count and
    its list cannot disagree. For the bar it reads only the keys of the named person and of their supervisees now
    (`Supervision::supervisees_now`); for the picker, every person's keys. The staff id is normalised by the database
    (`cairn_vault_staff_id`), never by a copy of its rule in Rust.

### The window (`person_menu.rs`, `person_menu_view.rs`, `person_menu.js`)

13. **Every sentence is Rust's.** `person_menu_view.rs` words the rows, the headings, "Nothing here.", the
    picker's suffix and the counts-failure sentence "Waiting counts could not be read." (sent as the error's text);
    `person_menu.js` writes no sentence a clinician reads. Each row's button shows "Open note" and is named "Open
    note: {patient}, {when}". A draft row reuses `notes::commit::due_words` verbatim. The menu's status line sits
    in the bar, outside the `<details>`, so a failed read is visible with the menu closed; a failed read also hides
    and closes the menu, so a list no longer known to be true is not left on the bar. The review group is a
    nested `<details>`, closed on every draw and never opened by the script.
14. **"Open note" re-reads, and keeps the wrong-chart rule.** A menu read admits its rows' charts to
    `AppState::shown`, as the duplicate tray does, and a chart opens only through `open_chart_impl`.
    `person_menu_open` re-reads the menu for the person named NOW and opens the chart only while a row of that read
    names it; otherwise it answers "That note no longer waits in your list — the list was read again." and opens
    nothing; a re-read that fails answers the read-failure text ("Your waiting list could not be read, so notes may
    be waiting for your signature: " and the cause) instead, and opens nothing either. The script first
    returns at once while a registration is under way, as `openChart` does, or while an earlier press of a row's
    button is still under way (a double-click); then lets a sign in flight settle, and if that Sign, Discard,
    sign-off or review did not go through (or is not known to have), stays on its chart, whose note pane says how it
    went, and says it on the bar's line in the who's-here switch's words; then closes an open chart (decision 3). On
    a refusal it reads the list again, then says the sentence (only if that re-read succeeded, so a failed re-read's
    line is not overwritten) and moves focus to it. The opened
    note pane scrolls to and focuses the part (found through the compound's `shown_parts`) or the draft (or the
    editor, if this terminal holds it).
15. **When the menu is read again:** on a change of the named person (the last person's menu is first hidden, closed
    and emptied, so none of it shows under the next person's name while the read is on its way), on opening the
    menu, after each drawn note pane and each chart closed (only once the node's `close_chart` has succeeded: a
    failed close returns before the re-read), after a refused "Open note", after each supervision act that went
    through, and every 60 s while the menu is closed. An older answer never draws over a newer one (a revision
    counter per kind of read). The picker's counts are not read after a failed staff list.

### What this ends

16. **ADR-0092 (d)** (no pending-sign-offs menu), **ADR-0093 (e)** (the parts to re-sign found chart by chart) and
    **ADR-0094 (n)** (finding what to co-sign chart by chart) end, for this node's charts (not guaranteed (c)), with
    one exception: a part that awaits a co-sign while its writer has no supervisor NOW is in nobody's group 2
    (decision 7), so such a part, a student's "— not co-signed (no supervisor recorded)" for one, is still found
    chart by chart (not guaranteed (k)). ADR-0092's (c), a repudiated signer left unmarked, had already ended with
    ADR-0093; this slice adds only the cross-chart list of such parts, which is ADR-0093 (e).

## What is not guaranteed

- **(a) The count is as fresh as its last read.** While the menu is closed it is at most 60 s old, and an act that
  draws a note pane or closes a chart re-reads it sooner (decision 15). While it stays open the tick is skipped, so
  a draft the quarantine commits then is not seen until a note pane draws, a chart closes or the menu is reopened.
- **(b) A draft on another node is not listed.** Drafts never sync (ADR-0087 decision 9). Today every terminal is a
  window on the facility node, so every draft there is listed; once slice 5 makes each terminal its own node, a draft
  on another terminal-node will not be ([#963](https://github.com/cairn-ehr/cairn-ehr/issues/963)).
- **(c) The menu spans this node's charts only.** A part or a vouch not yet synced here is missing or still listed,
  and the supervision record and a key's repudiation are node-local (ADR-0094 (a), ADR-0093 (i)), so another node's
  menu for the same person can differ.
- **(d) A draft row never says that the window's last commit try failed.** It is worded with `failed` false, and
  "overdue" is judged by the window's clock against the door's `due`; the chart's own draft row can say more.
- **(e) The 7 days are judged on the part's `hlc_wall`**, which a peer's clock can ratchet up to 24 h ahead
  (ADR-0094 (d)), so a note can stay in group 3 that long past its 7 days.
- **(f) "Open note" checks the chart, not the row.** It opens a chart while any row of the re-read menu names it,
  even when the pressed row itself has left the list.
- **(g) With nobody named, "Open note" says "the list was read again" though nothing was read.** A who's-here change
  hides the menu at once, so this is reachable only when the change lands while an earlier press of "Open note" is
  still under way.
- **(h) The ≤ 1 s budgets are unmeasured, and the reads grow with all-time history.** `NOTES_BY_KEYS_SQL` scans
  every `clinical_note` row's contributors and returns every part the keys ever wrote: groups 1 and 2 are all-time
  obligations, and only group 3 is cut to 7 days, in Rust. The picker reads every person's parts on each opening
  (decision 1). Both reads run on the window's one database connection, so a who's-here pick, a save or a Sign waits
  behind a long read; and "Open note" from an open chart reads the menu three times (after the chart closes, in
  `person_menu_open`, and after the note pane draws). RUNBOOK §26 owes the figures, and its fixture (about 60 notes)
  cannot show a cost that grows with history. The upgrade, a projection of open obligations, is
  [#1021](https://github.com/cairn-ehr/cairn-ehr/issues/1021).
- **(i) The window's commands against a database are pinned only by DB-gated tests**
  (`person_menu_db_tests.rs`), which CI does not run ([#905](https://github.com/cairn-ehr/cairn-ehr/issues/905)).
  The webview was walked in Chromium with a stubbed `invoke`; WebKit and the real Tauri IPC are owed to RUNBOOK §26.
- **(j) Two people rows can share a staff id, and then the bar and the picker can disagree.** After a restore loses
  the member rows and the person is enrolled again under another role (a new actor), both actors go by the same
  staff id once any supervision row names the old actor, and from then on (the record is append-only):
  `cairn_note_people()` lists an actor with no member row only when the record names it (db/073). The bar's read
  takes the first match in actor-hex order (`keys_for_bar`, `MenuRead::person`), while `counts()` emits one count
  per actor and the picker keeps the last one it is given ([#1020](https://github.com/cairn-ehr/cairn-ehr/issues/1020)).
- **(k) An awaited co-sign with no supervisor now is in nobody's menu.** A part that `awaits_cosign` while its writer
  has no supervisor recorded now (a student's "(no supervisor recorded)" part, or a required writer's whose
  supervisors all stepped away) is in nobody's group 2 (decision 7) and so in no count. It is found chart by chart.
  While its writer still needs co-signing and still has a member row, the assigner's panel shows them as a gap row
  (ADR-0094); a writer whose member row is gone is not on that panel at all, which lists members only
  (`cairn_supervision_staff()`, db/072).

## Consequences

- **Easier.** A clinician sees on the bar whether anything waits for their signature, and a supervisor finds every
  supervisee's note awaiting a co-sign without opening charts. Steps (paper *N* → built *M* → target *K*):
  - find what waits for my signature: one walk per folder → 2 (open the menu, read it) → 1 (the bar's count already
    says whether anything waits);
  - countersign one note from the menu: 2 (open the folder, sign) → 3 ("Open note", "Sign off this note" or
    "Co-sign as reviewed", the passphrase counted as slice 1 counts it) → 3, ruled not an `M > N` defect
    (decision 4);
  - audit a supervisee's recent work: pull files at random (unbounded) → 2 (open the menu, open the review group) → 2.

  Time budgets, owed by a human in RUNBOOK §26: the menu opens ≤ 1 s with 20 supervisees and a week of notes; the
  picker's counts land ≤ 1 s after its list draws; a pick made while they load puts the new name on the bar ≤ 1 s
  after the press; co-sign from the opened note ≤ 3 s (Sign's, RUNBOOK §16/§23).
  The menu is a read and db/044's timing kinds are writes, so the ≤ 1 s figures are stopwatch only. No decision is
  forced: the count names only obligations, and the review list is opt-in and collapsed.
- **Harder.** A new "is this owed?" rule must change the chart's predicate (`part_is_signed`, `counts_as_unsigned`,
  `awaits_cosign`, `reviewable`), never grow a menu-only copy. A column the chart's note read needs is added to
  `notes_sql!` once, for both reads. Every place that can change what waits must call `refreshMenuCount()`.
- **The bet.** That one list on the bar, counting only what must be signed, is enough for people to clear their own
  and their supervisees' notes without being nagged. We would know it is wrong if counts sit above zero for days with
  the person on shift, or if supervisors never open the review group.
- **Pinned by** `db/tests/073_person_menu_test.sql`; `crates/cairn-node/src/note/{menu_tests,menu_read,supervision,read}.rs`'s
  tests; `crates/cairn-node/tests/{note_menu_doors,note_parts_by_keys,note_person_menu,window_role_grants_notes,floor_execute_grants}.rs`;
  `cairn-gui/cairn-gui-tauri/src/{person_menu,person_menu_db,person_menu_webview}_tests.rs`. RUNBOOK §26 carries the
  human measurements.
