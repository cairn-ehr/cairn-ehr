# ADR-0087 — The first note stream and its draft

- **Status:** Accepted (partly supersedes [ADR-0041](0041-progress-note-narrative-format.md) decisions 1 and 2, and
  [ADR-0026](0026-node-durability-and-disaster-recovery.md) decisions 1 and 3 for note drafts)
- **Date:** 2026-11-03
- **Spec version at acceptance:** 0.89
- **Issues:** draft PR [#931](https://github.com/cairn-ehr/cairn-ehr/pull/931) (GUI sign-in slice 3a). Design:
  `docs/superpowers/specs/2026-11-03-sign-in-slice-3-notes-drafts-and-whos-here-design.md`; plan:
  `docs/superpowers/plans/2026-11-03-sign-in-slice-3a-note-stream.md`. Follow-ups: [#364](https://github.com/cairn-ehr/cairn-ehr/issues/364)
  (retire `note.added`), [#948](https://github.com/cairn-ehr/cairn-ehr/issues/948) (a `refs` target that arrives after
  its note), [#949](https://github.com/cairn-ehr/cairn-ehr/issues/949) (a media descriptor the escape changes),
  [#950](https://github.com/cairn-ehr/cairn-ehr/issues/950) (erasure does not reach an open draft),
  [#945](https://github.com/cairn-ehr/cairn-ehr/issues/945), [#946](https://github.com/cairn-ehr/cairn-ehr/issues/946),
  [#947](https://github.com/cairn-ehr/cairn-ehr/issues/947) (the editor's open edges),
  [#951](https://github.com/cairn-ehr/cairn-ehr/issues/951), [#952](https://github.com/cairn-ehr/cairn-ehr/issues/952),
  [#953](https://github.com/cairn-ehr/cairn-ehr/issues/953) (found by the final review).
- **Builds on:** [ADR-0041](0041-progress-note-narrative-format.md) (the note's wire format) ·
  [ADR-0052](0052-born-sealed-clinical-bodies.md) (born sealed) · [ADR-0053](0053-per-write-human-authorship.md) (the
  human signs, the node seals) · [ADR-0003](0003-bitemporal-time-and-acknowledged-uncertainty.md) (stated time) ·
  [ADR-0056](0056-unknown-event-types-admitted-uninterpreted.md) (custody total, power deferred) ·
  [ADR-0076](0076-duplicate-repair-a-linked-chart-reads-as-one-and-a-human-judgement-outranks-a-machine.md) (a linked
  chart reads as one) · [ADR-0084](0084-the-window-never-holds-the-owners-keys.md) (the window's own login)

## Context

ADR-0041 settled the note's wire format before any note existed. Slice 3a builds the first part of it: a clinician
presses **New note**, types, and signs, and the note is listed on the chart with its writer's grade. A crash or a
lost connection must not lose the text. The first signed `clinical.note.asserted` freezes its shape forever, so what
this slice put on the wire, and what it left out, has to be written down now.

Four things in ADR-0041 could not be built as written:

1. **Decision 1 retires `note.added` in the same migration.** That event is the general test fixture of dozens of
   suites and drives `patient_chart.note_count`. Retiring it would have doubled the slice, and no production fleet
   has ever admitted one.
2. **Decision 2's body has no recording times.** The design (§4.3) asks for three times per note: when writing
   began, when it ended, and when the writer says it happened. A nurse heads a paper entry "03:00" while writing at
   05:10, and both facts are true.
3. **The profile excludes things a clinician types every day.** `pt said "<b>"`, `[sic]: he said` and a markdown
   link pasted from a discharge letter all read as markup to a renderer. Refusing them would refuse the clinician for typing. Signing such text unescaped could put raw HTML
   or an external link under a signature forever.
4. **ADR-0041 names the §3.10 draft store but no draft store existed.** ADR-0026 had already promised that drafts
   survive a dead disk in the sealed local-state export (decisions 1 and 3).

The build took thirteen rulings (R1–R13) and the final review one more (R14), and the code's comments cite them by
number. This ADR is where they are recorded: each one that bears on the architecture is stated below under its number
(R2 was about process only).

## Decision

### The note as built (`db/064_clinical_note.sql`, `crates/cairn-event/src/note.rs`)

1. **The text-only subset of `clinical.note/1`.** The window writes exactly seven payload keys: `narrative`, `media` (`[]`),
   `refs` (`[]`), `encounter` (`null`), `recording_started`, `recording_ended` and `effective_basis`. The type is
   registered `additive`, never targeting another author. It is born sealed like every `clinical.*` type, and it
   carries the per-write human authorship of ADR-0053: the human signs, and the contributor set is
   `{human, authored}` + `{node, recorded}`. It stays off db/048's no-thread list, so it keeps the conservative
   sensitivity bound.

2. **Three new body keys: a partial supersession of ADR-0041 decision 2.** `recording_started` and `recording_ended`
   are RFC 3339 instants stamped by the node's database clock, never by the window. `recording_started` is when the
   node first stored the draft (the first autosave, or the Sign itself). `recording_ended` is the draft's last real
   change. The window autosaves on the **first keystroke** of a burst, then at most once a second while typing goes
   on, and once more after the last keystroke (ruling R14), so a new draft's first autosave is its first keystroke's
   and a power cut loses about a second of typing. A trailing-only debounce saved nothing until the typing paused.
   `effective_basis` is `stated` when the writer gave a time and `recording-started` when they did not. The envelope's
   `t_effective` is **always set**: the stated time, or `recording_started` (principle 4: "no stated time" is said,
   never passed off as a stated one; the window shows *no stated time · writing began 03:12*). The commit's HLC stays
   `t_recorded`. Decision 2's `narrative` + `media` body is otherwise unchanged.

3. **The floor (`cairn_check_note_assertion`) refuses only what ADR-0041 excludes forever, plus what it makes
   integrity.** Its rules, each with its own message:
   1. `schema_version` is `clinical.note/1` (see decision 6 for the remote door).
   2. `narrative` is a string that is not blank by `cairn_twin_is_present`'s Unicode-exact test (ruling R7).
   3. No unescaped raw HTML (`<` followed by a letter, `/`, `!` or `?`) and no unescaped email autolink (`<name@`).
      `BP <90` and `<5 min` pass.
   4. Every unescaped link or image target (`](…)`) is **exactly** `cairn:att/<id>)` with `<id>` drawn from
      `[A-Za-z0-9._~-]+` (ruling R5). A `cairn:event/` target is refused by name (ADR-0041 decision 9). Any other
      target is refused: an external URL is excluded forever.
      - 4a. No unescaped `]:` (ruling R3): a reference-style definition `[ref]: http://…` would carry an external URL
        past rule 4.
   5. `media` is an array, and must be empty at the authoring door. At both doors every anchor names exactly one
      manifest entry with a non-blank descriptor, and is written `![<descriptor>](cairn:att/<id>)` byte for byte.
      So a plain link to `cairn:att/` (`[x](cairn:att/m1)`) is refused (ruling R6): ADR-0041 decision 3's anchor is
      the image form only. At the authoring door, with `media` empty, any anchor is dangling.
   6. `refs` is an array. Each entry is `{rel, event}`, with `rel` one of `addendum-to`, `correction-of` or
      `transcript-of`, and `event` a uuid this node holds in `event_log`.
   7. The `encounter` key is present, and `null` at the authoring door.
   8. Both recording times are strings that parse through db/001's `cairn_t_effective`, the one wire-pin validator
      the envelope already uses (ruling R7): an explicit offset is required and `now` is refused. Started is not
      after ended.
   9. `effective_basis` is named. `t_effective` is set, and with `recording-started` it is the same instant as
      `recording_started`.

   The floor does **not** refuse what the v1 profile merely leaves out (tables, code blocks, footnote references).
   Those render as literal text, which ADR-0041 decision 5 already calls degraded but honest, and refusing them would
   make a clinician's pipe-separated vital signs unsignable. (A footnote *definition*, `[^1]: …`, has the shape of a
   reference definition and so falls under rule 4a unless it is escaped, as the window escapes it.) The rules that
   only this window's output obeys (`media` empty, `encounter` null) run at the authoring door only
   (`cairn.remote_apply` is not `on`, the db/031 idiom), so a future media-capable peer is never wedged (ADR-0056).

4. **The window escapes; the floor's refusals are for raw-SQL clients.** `escape_note_markup` writes `\` before every
   `<`, before a `(` or `:` that directly follows `]`, and doubles every existing `\`. It is applied once, at signing,
   to the text the draft holds; drafts hold the raw text. Escaped text never trips rules 3, 4 or 4a, so **a clinician
   is never refused for text that looks like markup**. `unescape_note_markup` removes a backslash before ASCII
   punctuation, and the window shows that: the text exactly as typed.

5. **The legibility twin is the unescaped narrative** (ruling R4). ADR-0041 decision 4 says the twin is "the
   narrative verbatim". For an escaped narrative, *verbatim* means the text as written: `render_note_twin` is
   `unescape_note_markup(narrative)`, so a text-only node, a screen reader and full-text search read `BP <90`, not
   `BP \<90`. This is frozen under every signature from the first note. The floor requires a non-empty twin (the #173
   registry row) but, as for medication, does not re-derive it.

6. **A newer profile is carried, never shown** (ruling R8). At the **remote** door a `clinical.note.asserted` whose
   `schema_version` is not `clinical.note/1` skips every profile rule: it is admitted, carried and forwarded byte for
   byte (ADR-0056; ADR-0041 decision 3's version-skew custody). Refusing it would wedge sync on every node older than
   its author. That is safe only because `clinical_note_apply` projects `/1` alone, and the window reads notes only
   from `clinical_note`, never from `event_clear` or `event_log`'s narrative. A peer that relabels a hostile body `/2`
   gets it carried, never on screen. The **authoring** door refuses anything but `/1`.

7. **The projection and its shred.** `clinical_note` holds one row per note, keyed by the event. A note is never
   overlaid: an addendum or a correction is a new note carrying a ref, so there is no winner rule. The applier reads
   the clear payload; a node without custody projects nothing, and the late-custody path re-runs it when a key lands
   (`heal_safe`). `cairn_execute_shred` (db/037, in place) deletes the row by `content_address`.

8. **A chart's notes read as one over a linked set** (ADR-0076), newest `t_effective` first. The node hands the
   window instants; the window words them in the terminal's own time zone (ruling R10). While the set holds a
   link this node doubts, a note on a chart other than the opened one is counted and withheld with a sentence, as the
   medication list does. The pane's notes and drafts are read in **one** repeatable-read, read-only snapshot
   (ruling R12): otherwise a sign committing between the two queries shows neither the note nor its draft. The
   vouched attester that grades authorship comes through a new definer door, `cairn_note_vouched_attester`
   (ruling R11), because `cairn_window` may not call `cairn_attestation_vouched` itself.

### The draft as built (`db/065_note_draft.sql`)

9. **A draft is not the record.** It lives in `note_draft`, a deliberately mutable table beside the append-only
   log. It is never synced or signed, and it is **not in any backup**: the backup captures the signed event planes
   only, and the sealed local-state export's draft slot stays empty. That partly supersedes ADR-0026 decisions 1 and
   3, which said drafts survive in the export: a dead node loses its unsigned note drafts. Draft prose is clinical
   text in the clear, and an export would carry it to every copy of every medium for the life of the medium, past
   any later erasure. A crashed window, the common case, still gets its draft back from the node.

10. **Definer doors only.** `note_draft` and `note_draft_spent` are revoked from PUBLIC and granted to no role. The
    three draft doors (`cairn_note_draft_save`, `cairn_note_drafts`, `cairn_note_draft_spend`), the byline door
    `cairn_note_signer_name` and `cairn_note_vouched_attester` are `SECURITY DEFINER` with
    `search_path = public, pg_temp`, revoked from PUBLIC and granted to `cairn_window` only. There is no per-save
    encryption: a draft at rest is no more readable than the clear view of the sealed event it becomes.

11. **One draft, one chart, one terminal.** A draft names its patient and the terminal that started it. A save of an
    existing draft naming another patient or terminal answers `elsewhere` and writes nothing. The terminal id is a
    UUIDv7 minted once and kept in the window's config directory. A save answers `saved`, `spent`, `elsewhere` or
    `no-chart`. `last_keystroke` moves only when the text or the stated time really changed (ruling R1), so an idle
    autosave does not move `recording_ended`. It is stamped `GREATEST(clock_timestamp(), recording_started)`, never
    earlier than writing began, and a new draft takes one clock reading for both times. A wall clock stepped back
    while a draft is open (an NTP step after a long offline spell) would otherwise store a last keystroke before the
    start, so the note signed from it would end before it began, which rule 8 refuses; and under R1 a Sign with
    unchanged text never restamps, so the draft would stay unsignable. The recorded end is then a near-truth, "no
    earlier than writing began", not an untruth (principle 4). In 3a the writer is not known until the note is signed;
    the §3.10 store keyed by `(author, patient)` is 3b's.

12. **A draft is spent by its own note, at commit.** Sign runs in **one** transaction: it saves the text sent with
    Sign (so the note is that text, never an older autosave), spends the draft, builds the body from the spent row,
    escaping it there, and submits it. A refused sign leaves the draft exactly as it was before the press. A deferred constraint
    trigger refuses, at COMMIT, a spend whose `clinical.note.asserted` of the same chart was not **inserted in the
    same transaction** (`event_log.xmin = pg_current_xact_id()`, ruling R9), and `note_draft_spent.event_id` is
    unique, so an older note can never justify wiping a draft. A row inserted in a subtransaction carries the
    subtransaction's id, so the sign path submits with no savepoint. A later save of a spent draft answers `spent`
    and writes nothing, so an autosave landing after Sign cannot bring the draft back.

13. **The editor is read-only while a Sign is in flight** (ruling R13). On paper the ink under a signature is fixed.
    Typed text that the sign would not carry must not appear to be kept.

### What is deferred

14. **`note.added` is not retired: a partial supersession of ADR-0041 decision 1.** The `clinical.note.asserted`
    registration landed without removing the walking-skeleton `note.added` row, its projections or its emitters. The
    retirement is tracked by [#364](https://github.com/cairn-ehr/cairn-ehr/issues/364), with `patient.amended`. No
    production fleet has ever admitted a `note.added`, so nothing in the field depends on the order. Decision 1 is
    otherwise unchanged.

15. **3b and 3c extend this ADR's shape, not its wire.** 3b adds who's here, the rest of the draft store (revisions,
    the writer set, per-writer parts committed as one event each), the commit runner after the quarantine timer and
    the audited discard. 3c adds addenda and "sign off this note". Each is recorded by its own ADR.

## What is not guaranteed

- **The terminal id is caller-asserted.** A `cairn_window` client on raw SQL can echo another terminal's id and
  overwrite that terminal's draft, or spend it with a note it signs in the same transaction. Prose is mutable until
  signed; 3b's revision rows will make earlier text recoverable. The id is a label kept stable, not a proof.
- **A terminal whose id cannot be kept** (an unreadable or unwritable config directory) falls back to an id for one
  session, so the window still opens. Its drafts read as another terminal's after a restart: visible and readable,
  not editable there.
- **Drafts are not backed up**, and **an erasure does not reach an open draft**
  ([#950](https://github.com/cairn-ehr/cairn-ehr/issues/950)): a shred works on an event, and a draft has none.
- **A `refs` target that arrives after its note at the remote door** is refused, because rule 6 needs the target
  held, and set-union sync does not deliver in order. Whether it should be penned instead is
  [#948](https://github.com/cairn-ehr/cairn-ehr/issues/948). It becomes live with 3c's addenda.
- **A media descriptor the escape changes can never match its anchor** (latent until media is authored,
  [#949](https://github.com/cairn-ehr/cairn-ehr/issues/949)). The anchor text sits in the escaped narrative, while
  the manifest descriptor is stored as typed. Rule 5 compares them byte for byte, so a descriptor containing `<` or
  `\` is always refused.
- **The id grammar of rule 4 is a wire commitment.** A future media slice wanting other characters in an attachment
  id must widen the floor, and an older node will refuse its notes at the remote door.
- **A `/2` note is held but not shown on a node that knows only `/1`**, and its refs and anchors are not checked
  there (decision 6).
- **The editor's open edges**: a stale "already signed" leaves the draft stuck until the chart is reopened
  ([#945](https://github.com/cairn-ehr/cairn-ehr/issues/945)); closing a chart while the node is unreachable loses
  the unsaved text ([#946](https://github.com/cairn-ehr/cairn-ehr/issues/946)); closing a chart waits on its saves
  with no bound or status ([#947](https://github.com/cairn-ehr/cairn-ehr/issues/947)).
- **Text containing U+0000 or a lone surrogate never saves or signs**
  ([#951](https://github.com/cairn-ehr/cairn-ehr/issues/951)): PostgreSQL text cannot hold U+0000, and the window's
  IPC boundary refuses a lone surrogate. The editor keeps the text on screen but says only "not saved yet".
- **A stated time later than the writing is accepted unflagged**
  ([#952](https://github.com/cairn-ehr/cairn-ehr/issues/952)): a date slip signs, and, listed newest `t_effective`
  first, tops the chart until 3c's corrections. ADR-0003's clash flag is owed.
- **Two windows on one machine share a terminal id** ([#953](https://github.com/cairn-ehr/cairn-ehr/issues/953)): it
  lives in the per-user config directory, so both windows take a draft as theirs, both reopen it, and their saves
  overwrite each other, which is what decision 11 exists to prevent.

## Consequences

- **Easier.** A clinician writes and signs a note at paper's step count (3 → 3 → 3), with nothing lost to a crash,
  and every text they type signs and reads back exactly as typed. The floor stays a short list of regexes and
  integrity checks, small enough to review. The twin, full-text search and the screen reader read plain text.
- **Harder.** Every reader of a note must read `clinical_note`, never the event's clear view (decision 6). The escape
  is part of the wire: every editor must escape what the floor refuses, and every reader unescapes by
  `unescape_note_markup`'s rule, which is CommonMark's backslash escape (a backslash before ASCII punctuation).
- **The bet.** That escaping, not refusing, is the right answer for text that looks like markup. We would know it
  is wrong if a later profile needs a construct the escape has already made inert in signed notes. Those notes would
  still read as typed, just not as the construct.
- **Pinned by** `crates/cairn-event/src/note.rs` (the escape round trip, exhaustive over a small alphabet, which `note_floor.rs`
  also runs through the real markup floor),
  `db/tests/064_clinical_note_test.sql` (every floor rule at both doors, R8), `db/tests/065_note_draft_test.sql`
  (the outcomes, R9, the grants, db/044's replay), `crates/cairn-node/tests/{note_floor,note_sign,note_read,window_role_grants_notes}.rs`
  and `cairn-gui-tauri/src/{notes,notes_webview}_tests.rs`. RUNBOOK §18 carries the human measurements.
