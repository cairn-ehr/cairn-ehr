# RUNBOOK §18 in the real window — a progress note (slice 3a, PR #931) — 2026-11-03, PARTIAL

**Status: partial.** The maintainer ran the first half of §18 in the real Tauri/WebKit window and stopped for the
night. Everything not ticked below is still owed. Nothing here moves a budget.

## Rig

| | |
|---|---|
| Host | MacBook, macOS (Darwin 27.2), `AppleLocale` `en_US@rg=au`, primary language en-US |
| PostgreSQL | 18 (Postgres.app), throwaway cluster on :5533, `scram-sha-256`, zone Australia/Brisbane |
| Window | `target/release/cairn-gui-tauri`, on the `cairn_window_login` role (ADR-0084), PR #931 head `56ee85f9` |
| Signer | `nurse-a`, enrolled from the window |

## Results

- [x] **The setup hook filled the terminal id.** The first launch minted one (`~/Library/Application
      Support/org.cairn-ehr.med-list/terminal-id`), and a note typed straight after launch saved. Saving and signing
      both reached the node, so Tauri's argument casing for `note_draft_save` / `note_sign` holds.
- [x] **Sign without a credential** is refused with a sentence; the text stays in the editor.
- [x] **Sign is instant** to the eye. The node's write half of the gesture (`ui_gesture_timing`, `note-sign`, bucket
      `1-3`): n = 4, p50 = p95 = **10 ms**. The ≤ 3 s stopwatch figure, Staff ID keystroke → outcome, was not taken.
- [x] **Signed by nurse-a**, listed with that name.
- [x] **No stated time:** basis `recording-started`, `t_effective` = `recording_started` (19:48:43);
      `recording_ended` = the last keystroke (19:55:55), not the press of Sign.
- [x] **The markup line** `BP <90, [L] arm (R), see [x](http://example.org) \ ok` reads back exactly as typed. Stored
      escaped (`BP \<90, [L] arm (R), see [x]\(http://example.org) \\ ok`), the twin unescaped. It was pasted, so
      `recording_started` = `recording_ended` (correct: one change).
- [x] **A stated time:** basis `stated`, `t_effective` 20:00 while writing ran 20:39:19 → 20:39:31; the list is
      ordered by `t_effective`.
- [ ] **Read-only while signing: INCONCLUSIVE, re-run.** With `note_draft` locked (`LOCK TABLE … ACCESS EXCLUSIVE`,
      10 s), the maintainer saw no greying and the sign "went straight through". The node's times say the press came
      after the lock was released: the draft's first save executed at 20:44:35.90 (it had been waiting on the lock),
      and the sign's HLC tick, which the lock does not block, is at 20:44:36.16. So the press did not overlap the
      lock. Re-run: type the note and fill the strip FIRST, wait two seconds, then lock for about 14 s and press Sign
      at once.
- [ ] Measurement 1 (open the editor ≤ 1 s; nothing saved until typed), the partly typed time, stated seconds after a
      reopen, all of Measurement 3 (hard kill, a kill within a second of a keystroke, Postgres stopped, another
      terminal's draft), keyboard only and VoiceOver (including whether the sign sentence is read twice).

## Findings

- **#954**: the signer strip has to be a fixed bottom bar that is always visible. Scrolling down to sign is not
  acceptable.
- **#955**: the `datetime-local` picker shows the date month-first. This is the Mac's language (en-US) winning over its
  region (AU). Fix it by making the date unambiguous: month as a word, 24-hour time.
- **Seen during the lock, not yet filed.**
  - An autosave blocked behind a lock tells the clinician nothing: no "saving…" and no "not saved yet". This is close
    to #947's unbounded wait.
  - `recording_started` is stamped when the first save *executes* on the node, not at the first keystroke. A save
    delayed by a lock or a slow link therefore moves "writing began" later.

  Both should be filed or folded into #947 at the next session.
