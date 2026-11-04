# HANDOVER — Cairn

## ⇒ NEXT

> [!IMPORTANT]
> **⇒ THE NEXT GOAL (maintainer, 2026-10-31): a GUI milestone** — register a clinician as a user, log in, search /
> select / register a patient, enter demographics, write a progress note, record an allergy, chart a medication,
> sign everything off, log out. Four decisions taken in conversation: (1) **a superuser role assigns new users**, at
> least one created at installation; (2) **a key vault holds the clinicians' signing keys** (login = "choose who you
> are + passphrase") — both built, ADR-0083; (3) **a draft auto-committed after the quarantine is RECORDED, NOT
> ATTESTED** — built, [ADR-0089](spec/decisions/0089-the-quarantine-commit-and-the-asserted-grade.md); (4) **"prescribe"
> in this milestone means CHARTED** (printed and electronic prescriptions come much later) — still in no ADR.
>
> **Its sign-in / authorship part:** design
> [`superpowers/specs/2026-10-31-gui-sign-in-and-authorship-design.md`](superpowers/specs/2026-10-31-gui-sign-in-and-authorship-design.md)
> (builds ADR-0008 §5.11; slices 1 sign-in → 2 re-key (#247, #664 first) → 3 drafts + `Asserted` grade + its display →
> 4 emergency key → 5 autonomous terminal nodes that sync credentials with specified nodes).
> - **Slice 1 MERGED 2026-10-31** (PR [#897](https://github.com/cairn-ehr/cairn-ehr/pull/897), [ADR-0083](spec/decisions/0083-the-human-key-vault.md),
>   `db/061`, `cairn_pgx` **0.4.0**): the key vault, per-act signing (`--key-hold-seconds`, 0 = shared terminal), the
>   signer strip, the terminal lock, "Enrol staff", the `vault-*` console verbs, one `EventSigner` seam.
> - **Slice 2 MERGED 2026-11-02 — PRs [#917](https://github.com/cairn-ehr/cairn-ehr/pull/917) +
>   [#920](https://github.com/cairn-ehr/cairn-ehr/pull/920)** ([ADR-0085](spec/decisions/0085-in-person-enrolment-and-re-key.md),
>   v0.87, `db/063`): an *enroller* enrols or re-keys IN PERSON, *unconfirmed* (never blocks signing) until a superuser
>   confirms or repudiates; `rotate-key` keeps the actor (#247 decided; #664 + #666 closed). **⚠️ Ruling R4:** remote
>   apply admits any key in `actor_key_vouches`, so a leaked, rotated-away, UNrepudiated key could inject events via a
>   peer or medium. **⚠️ A local-state export by this build will not decode on an older build.** Open: **#918** ·
>   **#921** (a decision) · **#922** · **#923** · **#925** · **#927** · **#929**. #919 + #924 accepted, #926 decided in
>   [ADR-0086](spec/decisions/0086-who-may-act-after-a-repudiation.md) (v0.88, PR #928; the console verb
>   `vault-repudiate-superuser-key`). Owed by a human: **RUNBOOK §17**.
> - **Slice 3a MERGED 2026-11-03 — PR [#931](https://github.com/cairn-ehr/cairn-ehr/pull/931)**
>   ([ADR-0087](spec/decisions/0087-the-first-note-stream-and-its-draft.md), v0.89, `db/064` + `db/065`): the note
>   stream and its draft. **What a later author must know** (ADR-0087 has each): the window ESCAPES markup once, at
>   signing, and the twin is the UNESCAPED text (R4); the attachment-id grammar is a wire commitment (R5); **read notes
>   only from `clinical_note`** (R8); a spend names a note INSERTED IN THE SAME TRANSACTION, so no savepoint (R9);
>   drafts are in no backup or export; `note.added` stays (#364). Open: **#945** (half built in 3b-1) · **#946** ·
>   **#947** · **#948** · **#949** · **#950** · **#951** · **#952** · **#953** · **#957** (a decision) · **#959** ·
>   **#960** · **#961** · **#962**. **RUNBOOK §18 HALF RUN** 2026-11-03
>   (`cairn-gui/cairn-gui-tauri/results/2026-11-03-note-stream-human-pass.md`): filed **#954** (the signer strip must be
>   an always-visible bottom bar) and **#955** (an unambiguous date). Still owed: the read-only-while-signing re-run,
>   open ≤ 1 s, the partly typed time, stated seconds, Measurement 3, keyboard + VoiceOver. To file or fold into #947:
>   a save blocked behind a lock is silent; `recording_started` is stamped when the first save executes.
> - **Slice 3b-1 MERGED 2026-11-04 — PR [#966](https://github.com/cairn-ehr/cairn-ehr/pull/966)**
>   ([ADR-0088](spec/decisions/0088-whos-here-and-a-drafts-parts.md), v0.90, `db/066`): **who's here** (a claim,
>   never a sign-in; a pick and the idle re-lock forget the held key; claimed names `?Nurse A`, spoken "unverified");
>   **per-writer draft parts** with revision rows; **"Continue here" MOVES a draft** (`moved` split from `elsewhere`,
>   #958); **Sign = one event per non-blank part**, chained by `addendum-to`, so **⚠️ #948 is LIVE** (part 2 can reach a
>   peer before part 1). Open: **#963** (slice 5) · **#964** · **#965** · **#967** · **#968** · **#969** · **#970** ·
>   **#971** · **#972** · **#975** · **#976** · **#977** · **#978**; #973 built. Owed by a human: **RUNBOOK §19**.
> - **Slice 3b-2a MERGED 2026-11-04 — PR [#981](https://github.com/cairn-ehr/cairn-ehr/pull/981)**
>   ([ADR-0089](spec/decisions/0089-the-quarantine-commit-and-the-asserted-grade.md), v0.91, `db/067`, db/065 + db/066
>   in place): a draft untouched for the site's quarantine (`cairn-node note-set-quarantine`, default 180 min, console
>   only) goes in **unsigned**, one node-signed event per non-blank part, `{writer, drafted}` + `{node, recorded}`; a
>   keyless named writer leaves it **stuck**; `Asserted` reads "written by ?Nurse A · went in unsigned"; a 60 s Rust
>   tick plus a commit before every chart read; a draft whose last commit failed says so. **#974 ruled not `M > N`**
>   (closed). Filed **#979** (deferred) · **#980** · #982 (built, closed) · **#983** (residual minors); commented #968.
>   Owed by a human: **RUNBOOK §20**.
> - **Slice 3b-2b BUILT 2026-11-05 — draft PR [#984](https://github.com/cairn-ehr/cairn-ehr/pull/984)** (branch
>   `plan/sign-in-slice-3b-2b-audited-discard`; [ADR-0090](spec/decisions/0090-the-audited-discard.md), spec **v0.92**,
>   `db/068` + `db/069` with `db/037`, `db/044`, `db/065`, `db/066` edited in place, `SCHEMA_GENERATION` **69**; plan
>   [`superpowers/plans/2026-11-04-sign-in-slice-3b-2b-audited-discard.md`](superpowers/plans/2026-11-04-sign-in-slice-3b-2b-audited-discard.md)).
>   **Discard draft** beside Sign, same strip, no dialog: a signed, sealed `clinical.note-discard.asserted` keeps what
>   the draft said; only a draft whose every non-blank part is the discarder's own **by key** (or by a writer not
>   known) may go; db/065's trigger holds the spend to an event signed by the discarder **at the instant the door
>   minted under the lock**, so neither a colleague's key id nor a captured discard can be replayed; save/take answer
>   `discarded`, never `spent`; an all-blank draft is deleted with no event. The chart lists a one-line `<details>`
>   marker, never a note (maintainer, 2026-11-04). Open: **#987** (a discarder's own raw-SQL client can sign different
>   text; filed by the ADR) · **#985** (a raw-SQL Sign spends a colleague's draft) · **#986** (Sign's unknown-outcome
>   lead) · #980 still. Owed by a human: **RUNBOOK §21** (discard ≤ 3 s; the colleague's-draft and stuck refusals; the
>   marker by keyboard + VoiceOver; WebKit).
>
> **#898 + #899 MERGED in PR [#911](https://github.com/cairn-ehr/cairn-ehr/pull/911)** (2026-11-01; design
> `superpowers/specs/2026-11-01-window-connection-posture-898-899-design.md`, plan of the same date,
> [ADR-0084](spec/decisions/0084-the-window-never-holds-the-owners-keys.md), spec **v0.86**, `db/062`,
> `SCHEMA_GENERATION` **62**). The window runs on its own login (`cairn_window`: inherits `cairn_agent`, never
> `cairn_node`), never loads the schema (a stamp mismatch refuses the launch), refuses a database on another machine
> (no TLS exists, so no override), and `release_for_act` refuses — before any passphrase is sent — over a connection
> that is over-powered through ANY role it is a member of. **`cairn-node provision-window-role --role <name>`** is the
> one owner step for install AND upgrade; it refuses a cluster with any `trust` rule or an unreloaded `pg_hba.conf`.
> **⚠️ RUNBOOK §0 changed:** the measurement cluster must use `scram-sha-256` (provisioning refuses `trust`), and every
> window launch uses `$WINDOW_CONN`. Tests are unaffected (they log in as a probe role, never through the verb).
>
> > [!WARNING]
> > **NOT YET GUARANTEED — gates on any pilot:** until **#909** (pinned, installer-generated TLS) or slice 5 (every
> > terminal its own node) lands, a facility has ONE usable terminal: the one on the database's machine. **#912**: no
> > long-running process re-adjudicates deferred events (the window's launch used to, incidentally). **#910**: a
> > Cairn-installed Postgres (brainstorm first). From PR #911's second review: **#914** (say "cannot sign" at launch, not at the first signature) · **#915** (the posture check's routes are a hand-kept list) · **#916** (small robustness residuals); #913 was fixed in the PR. **Slice 5's design must answer** (design §4; and since 2026-11-04 how a draft reaches the facility's other terminal-nodes, partitioned case included — drafts never sync, **#963**): synced sealed blobs on a
> > stolen terminal disk can be guessed offline, and the attempt counter is per node. Open from slice 1: **#900**
> > (a superuser key could sign a clinical event) · **#901** (bound stored Argon2 parameters) · **#902** (handle case) ·
> > **#903** (another's guesses can hold a member at the 300 s cap — a decision) · **#904** (an unreadable blob counts
> > as a wrong guess; `cairn_pgx` 0.5.0) · **#905** (CI never runs `cairn-gui-tauri`'s DB-gated tests — run them
> > locally) · **#906** · **#907** · **#908** (a `cairn_pgx` reinstall re-grants `cairn_vault_unseal` to PUBLIC unseen).
> > Human-key `&SigningKey` parameters slice 5 must move onto the seam: `identify.rs:83`, `apply_proposal.rs:94`.
> > **Local `cairn_pgx` reinstall on this Mac:** patch the dylib with `scripts/macos-patch-pgx-dylib.py` (CI unaffected).
> > **Key-hold terminals:** the strip keeps the last staff ID, so a next person typing only a passphrase costs the
> > PREVIOUS person a counted try (never a wrong author) — revisit with slice 3's "who's here" (its design: switching forgets the held key).

> [!NOTE]
> **Merged 2026-10-18 → 10-31 (ROADMAP has each; durable-rules has their blocks):** PR #895 (#893: a cut-off
> command says the window cut it off, `ConnectionGuard::explain`, pinned by `node_lock_explains_tests.rs`) · #892
> (#875 option 2, the 15 s cut + waiting bar; its brainstorm filed **#891**, its review **#894**) · #890 (#810) · #889
> (#884 + #888) · #887 (#882 + #885) · #886 (#878) · #883 (#880 + #881) · #879 (#876) · #877 (#875 option 1) · #874
> (#807; RUNBOOK §14 (c) human-owed) · #873 (#855 + #837 item 1, ADR-0082 E1) · #872 (#869) · #868 (#861; filed
> **#870**, **#871**) · #865 (#863 + #864; filed **#866**, **#867**) · #860 · #856 · #853
> ([ADR-0082](spec/decisions/0082-a-release-of-an-ended-hold-is-refused-at-the-local-door.md), spec v0.84) · #851 ·
> #847 · #845 · #842 · #838 (the window's **W**; **owed by a human: RUNBOOK §15** — W ≤ 10 s, Release ≤ 5 s,
> VoiceOver/keyboard) · #831 ([ADR-0081](spec/decisions/0081-medication-withhold-is-a-time-bounded-hold.md), `db/059`)
> · #830 · #829 · #827 · #824 · #821 · #819 · #816 · #813 · #805 (#797, "Read this chart again"; RUNBOOK §14 is its
> owed timing) · #802. Still open from them: **#858** · **#850** · **#837** · **#825**
> · **#185** · **#862** (`run-db-gated-tests.sh` with `CAIRN_TEST_PG*` but no `PGPORT` runs its SQL step against
> :5432 — pass `PGHOST=127.0.0.1 PGPORT=5532`). The repair path (#679 · #680 · #681)
> is built through R7. Design `docs/superpowers/specs/2026-09-27-duplicate-repair-path-679-680-681-design.md`; R4's
> runbook `docs/developers/running-the-duplicate-check.md`; human passes RUNBOOK §11–§14. spec **v0.84** (newest
> ADR-0082). DOB-near-miss follow-ups: #759 · #760 · #762 · #764 · #765 · #766. Trap: a new blocking key can generate
> a pair an older test asserts absent — fix the fixture date, never the assertion.
>
> **⇒ NEXT, in order:**
> 0. Check `gh pr list` before trusting this list (house rule 8).
> 1. **3b-2b BUILT — the maintainer reviews and merges draft PR [#984](https://github.com/cairn-ehr/cairn-ehr/pull/984)** (above). **Then 3c** (addenda to signed notes, "sign off this note", the compound note, a vouch that upgrades
>    `Asserted` — it must answer #948 too). Design §10's findings 1–2 are decided (ADR-0088), 3 is deferred (#979),
>    4–5 are **#965**, **#964**.
>    Design: [`superpowers/specs/2026-11-03-sign-in-slice-3-notes-drafts-and-whos-here-design.md`](superpowers/specs/2026-11-03-sign-in-slice-3-notes-drafts-and-whos-here-design.md)
>    (PR #930, merged; it also carries the per-note "unconfirmed" marker and #918's display half). The maintainer picks
>    the order of **#909** vs slice 5 (either gates a multi-terminal pilot) and of **#912**.
>    Owed by a human: switch the measurement cluster to `scram-sha-256` before the next RUNBOOK run (§0); **RUNBOOK
>    §16** (sign ≤ 3 s, next-nurse switch, VoiceOver/keyboard on the strip, the terminal lock), **§17**, **§18**, **§19**, **§20**, **§21**; #838's
>    **§15** (incl. #861's current-line check); **§14** (c) (the cause on screen; a refused reopen offers no retry; the
>    front door's top line clears) and (d) (no waiting bar on an ordinary press; a locked `medication_statement` →
>    "Waiting…", cut off by 15 s, and since #893 the outcome says the window cut it off). **#894** needs the satellite
>    rig; #891 needs a brainstorm before any build. Decisions owed: **#870** (the wrong-chart warning names the dose) ·
>    **#871** (a current line's angioedema shows only on hover — paper-parity) · **#866** (an earlier-shredded plaintext
>    event comes back on a heal or re-fold) · **#867** (should a failed re-fold block the erasure) · **#825** (where the
>    window's operator diagnostics live) · **#850** (acknowledging a dealt-with report) · **#837** item 3 (shredding a
>    release brings its hold back; items 1–2 done) · #834 (its shape). **#858** (a peer release on a sibling thread is
>    shown nowhere). An idempotent re-send of a registration (`submit_event` is a no-op on a repeated `event_id`) is
>    possible now that the window reopens a lost connection (#807), with #649. **#728** (the Pi re-measure) can run
>    beside. #699 stays open until the maintainer closes it.
> 2. **Decided in the #797 brainstorm (2026-10-17):** #803 (slice A PR #831, slice B PR #838, both merged; follow-ups #833 ward-level
>    list, #834 no marker for an undecryptable event) → **#804** (on link, show only where the two charts' medication
>    lists DIFFER, with continue / discontinue / withhold per line; needs #803 and **#690**). From #797's reviews:
>    **#812** (window type/test tightening) · **#808** (two outcome-line races) · **#809** (window tech debt).
>    **Open repair-path issues:** #712's items 1, 2, 4, 5 · from R7 **#790** · **#791** (both decisions) ·
>    from R6 **#784** · **#785** · from R5b **#750** · **#751** · **#753** · **#755** · **#756** · **#757** · from R5a
>    **#738** · **#739** · **#746** (a decision) · **#747** · #722 · from R4 **#730** (a decision) · **#731** ·
>    **#734**. Earlier: **#709** · **#710** · **#712** · **#714** · **#715** · **#718** · **#719** · **#720** ·
>    **#699** · **#703** · **#705** · **#706** · from R1: #690 (a decision) · #691 · #692 · #693 · #694 · #695 · #696;
>    #333, #220 and #335 gained comments.
> 3. **Human acts still owed** — see *Four things still owed* below; the repair path adds a **linked chart's open**,
>    **runbook §10's unlink** (≤ 15 s), the **live Tauri-IPC + VoiceOver pass on a linked pair** (the #699 (a) third-chart
>    unlink, a doubted pair's withheld lines, R3's person row), **RUNBOOK §11** (R5a: Review → judgement ≤ 20 s; a chart
>    open with the banner ≤ the single-chart open; the banner's VoiceOver checks), **RUNBOOK §12** (R5b: tray →
>    judgement ≤ 25 s, tray closed and open; the find ≤ 5 s with the tray OPEN; the tray's VoiceOver/keyboard checks),
>    **RUNBOOK §13** (R6: confirm a doubted link and re-link a bridged separation ≤ 15 s each; the front door's doubted
>    row; VoiceOver/keyboard — `TEMPLATE.md` has no §13 section yet), and R4's **running a supervised worker** on a real
>    node per the runbook (launchd/systemd; one worker per node; its role's `statement_timeout` ≥ 30 s).
> 4. **#620**, the only open item that can still change the wire (brainstorm first). Then **#769**, **#652 + #655**,
>    the advisory-tier **#640**, **#641**, and **#683–#686** (#685 needs the maintainer's permission to clear
>    `cairn_test`'s fixtures).
>
> **⇒ DURABLE RULES — READ BEFORE TOUCHING THE CODE THEY NAME:**
> [`developers/durable-rules.md`](developers/durable-rules.md) carries every slice's "do not undo" block (sign-in 3b-2b, 3b-2a, 3b-1, 3a, 2, 1, #898/#899, #745·#752·#740, R7, R6, R5b,
> R5a, #725, R4, R3, R1b, R2a, R2b-1, R2b-2, R1, the funnel) and the **eighteen traps**, each with its pin and its
> tempting wrong fix. A slice that adds rules adds its block there, not here.
>
> **Open from the funnel run:** #355 · #645 · #647 · #649 · #650 · #652 · #655 · #656 · #657 (multi-event rollback
> untested in both trees) · #658 · #662 · #663 · #664 · #665 · #666 · #667 · #668 · #669 · #670 · #672 (identifier
> entry) · #673 (the header shows age, not DOB) · #676 (the clerk reads `operator_chain` text) · #682 (an NFD trailing
> accent is lost — fixed in PR #776) · #683 · #684 (`1980-3-7` misses `1980-03-07` in db/046's DOB pass — a SET gap) ·
> #685 · #686.
>
> **⇒ THE NODE PLANE AND DR ARE CLOSED OUT; NO DECIDED-AND-UNBUILT ITEM REMAINS THERE.** Newest first:
> #621 ([ADR-0074](spec/decisions/0074-a-deterministic-door-failure-is-a-refusal-not-a-fault.md)), #619
> (ADR-0073), #614 + #615 (ADR-0072), #594 (ADR-0071: `restore` exits **3 INCOMPLETE**, **1 = BLOCKED**),
> #584 (ADR-0070), ADR-0069, the §1.2 restore measurement (100 003 events in 116.7 s against 600 s;
> **#512** stays open, `M > N`), DR slices 2c/2d (ADR-0067/0068). A solo clinic can lose its disk,
> restore, **open a chart** and rehearse it unattended. The durable rules are traps 1–18 in `developers/durable-rules.md`.
> - **⚠️ Citation discipline.** Rows coming back is NOT a body opening: cite
>   `restore_reads_the_clinical_plane.rs` and `restore_cli_surface.rs::a_scripted_restore_brings_the_clinical_record_back`,
>   never `dr_clinical_guarantee_gap.rs`'s counts. **Never cite ADR-0026 decision 1's promise 2** as met: no
>   node-default key tier exists (ADR-0067).
> - **The pen-release rule (#578):** a pen row carrying a wrapped DEK is released only when custody for its event is
>   SETTLED (`cairn_release_pen_row`, db/052; `pen_rows_leave_through_one_door.rs`). #585: nothing reads Postgres
>   notices. **`verify-backup` (#567)** fails `backup SHORT` only on evidence; operators run it AFTER `backup`.
>   Residuals #551 · #589 · #590 · #591 · #592 (#553 fixed in PR #774).
> - **#527/#562's triage note is false**; the real fix is **#575** (the minted recovery code reaches stderr). A retry
>   after a crashed restore must move the installed `<key>.unwrap` aside first (#596).
> - **Open decisions (none a patch):** #575 · #611 · #613 · #620. **Restore residuals:** #617 · #596–#599. **PR #601's
>   wave:** #605 · #606 · #607 · #609 · #610. **Node plane:** #268 · #301 (both `loop:needs-human`). **From
>   #619/#621:** #622 · #624 · #625 · #628 · #629 · #631 · #632 · #633 · #634. (Fixed by the 2026-10-12/13 cloud passes:
>   #602 · #603 · #604 · #608 · #616 · #569 · #626.) **Search:** #636 · #637
>   (the materialised token table — the right fix for the ~860 ms Pi floor, its own slice) · #639 · #640 · #641 · #643.
> - **Still broken, named rather than assumed away:** #549 · #552 · #502 item 4 · #101 items 2–3 · #583 · #586 ·
>   #587 · #556–#563 (ranges absorb issues that leave them; #536 is closed).

> [!WARNING]
> **⇒ CODEQL: ZERO OPEN ALERTS (measured 2026-09-12), KEPT THAT WAY BY A MODEL PACK — AND ONE HUMAN
> ACT IS DUE: make `CodeQL (rust)` a required check (#444).** PR #576 replaced default setup with a
> committed workflow + model pack (`.github/codeql/packs/cairn/codeql-models`, one `barrierModel` row per
> NAME-heuristic source, each with its reason). The flip needed the **organization-level** configuration as
> well as the repository's. **Read the alert list with `scripts/codeql-alerts.sh`, never assume it.**

> [!IMPORTANT]
> **⇒ GITHUB CLOSES ON ADJACENCY, NOT ON SENTENCES (2026-09-04).** Seven issues (#101, #115, #434,
> #441, #468, #500, #534) were closed by prose *disclaiming* the close; all reopened. Guarded by
> `scripts/check_closing_keywords.py` + `.github/workflows/closing-keywords.yml` (**#444** would make it
> required). **`fix(#500):` is SAFE** — the parenthesis breaks the adjacency. Run the script BEFORE you
> commit: the slip lives in the commit message and fixing it needs a force-push. Residuals **#547**, **#548**.

**The §5.9 thread ([#232](https://github.com/cairn-ehr/cairn-ehr/issues/232)): parts A+B (authority floor + operator
surface) BUILT, enforcing nothing beyond display/emission; C+D DESIGNED, C1 the next §5.9 BUILD.** Read
**ADR-0062/0063/0064/0065** first. The authority floor is ONE predicate `cairn_claim_authority` (db/005) at ONE site
(db/048's `NOT EXISTS`) — **#245**'s first SQL counterpart, not its mirror; operator-surface §1.2 budget met (residual
**#436**). **C+D (ADR-0065)** are a custody ladder (admission → named nodes → named actors) under one invariant:
**narrowing changes the cost and noise of reading, never whether content can be REACHED** (audited break-glass at every
rung; rung-1 glass is a NETWORK act, **#498**). The node's DEK is the keyring, the floor the glass (LOCAL); custody
composes by INTERSECTION, which can EMPTY (**#499**); it narrows on `event`/`patient`, never `thread`. **C1** = rung 1
(`custody.nodes`, both doors, serve-door withholding) + audited break-glass + in-chart location signal; rung 2 is
**#496** (needs a reader identity, §5.11). Related: **#494** (ADR-0052's `event_dek` sentence vs the built table),
**#377**, **#235** (shred authorization hooks), **#236** (FTS/RAG must build on `event_clear`).
**Two §5.9 facts that outlive their slices.** `REVOKE SELECT (column)` is inert while a table-level grant stands, so
`cairn_agent` holds an explicit 23-column grant on `event_log` omitting `safety` — a new column must be granted in db/049
§8 (`safety_read_grants.rs`), and that grant is cost-raising, not a floor (**#425**, **#427**; **#432** asks whether a
node should attempt one at all). Slice-65 follow-ons: **#374**, **#378** (withdrawal rationale is clear text forever —
the UI must warn today), **#379**, **#436** (#374/#379 each need a DECISION). The `arrayref` residue is **#454**.

> [!IMPORTANT]
> **Two code traps that outlive their slices, because both look like tidy-ups.** (1) **`content_address IS NOT
> NULL` is the "did anything win" test — never `subject_kind <> 'none'`** (`none` is a legal open-vocabulary value;
> ADR-0062 E6). (2) **Unknown ranks MAX in `db/048`/`db/049`, inverting `db/040`'s `ELSE 0`.** There rank 0
> withholds reject power (safe); here it would withhold protection. ADR-0065 adds a third member that agrees for a
> DIFFERENT reason (it withholds quiet access, reachable by break-glass) — do not carry that justification into a
> site where reachability is not guaranteed.

**Four things still owed are HUMAN acts an agent cannot do:** (1) **the §1.2 stopwatch figures** — follow
[`cairn-gui/cairn-gui-tauri/results/RUNBOOK.md`](../cairn-gui/cairn-gui-tauri/results/RUNBOOK.md) into a dated
`TEMPLATE.md` copy: sections 1–7 for the med-list gestures (only the *write* half is measured, median 222 ms,
**PARTIAL**; write-cost half **#360** unwired; also a **linked chart's open**, budget ≤ the single-chart open) and
**section 8 for the front door** (find ≤ 5 s, register ≤ 20 s, live and `--mock`; db/044's `gesture_kind` CHECK still
refuses a registration timing row until widened); (2) **the accessibility pass** — a live VoiceOver run through the
runbook's checks, front door included, keyboard-only, and on a **linked pair** (the source-chart label, #691; R3's
person-row label); DOM assertions automated by **#332**; (3)+(4) **make CI jobs REQUIRED status checks** (**#444**,
admin-only — "clippy + cargo test (cairn-gui)", "cargo doc (API surface)", and `CodeQL (rust)`). **If a measurement
falls outside its budget, that is the finding — file an issue, never adjust the budget.**

**Other build candidates** (nothing blocks a choice): the **drugref term→anchor lookup** (the §9 advisory tier; closes
the coded↔uncoded case ADR-0059 decision 5 leaves open; needs a connection-model decision; `safety_class_map` its empty
seam) · **the node/actor plane's two divergences** — db/007 fail-closes on an unmappable type (**#301**), and the node
puller skips-and-advances a verifiable refusal where the clinical one pens it (**#268**); both `loop:needs-human`.

**Standing gate:** whole-project review cycles repeat periodically; no release for clinical use before repeated cycles
pass cleanly. Last full pass 2026-07-15 (#187–#217), fully closed; the runnable clinical surface has never been
through one — include it next.

> [!TIP]
> **The tech-debt loop is stopped, and stays stopped** (maintainer decision, 2026-08-09) while a human session
> holds the main repo — they contend on one cargo lock and one `test_serial_guard` advisory lock. **A live IDE
> contends the same way** (rust-analyzer holds `target/`); use a scratch `CARGO_TARGET_DIR=/tmp/…`, never kill
> the IDE. Loop gaps **#326**, **#312**, **#322**.

---

**Session date:** 2026-11-04/05 (3b-2b plan → SDD, Tasks 1–7, draft PR [#984](https://github.com/cairn-ehr/cairn-ehr/pull/984)) · 2026-11-04 (3b-2a plan → SDD, Tasks 1–5, draft PR [#981](https://github.com/cairn-ehr/cairn-ehr/pull/981)) · 2026-11-04 (3b-1 plan → SDD, PR [#966](https://github.com/cairn-ehr/cairn-ehr/pull/966)) · 2026-11-03 (RUNBOOK §18 half run in the real window, #954 + #955) · 2026-11-03 (slice 3a built by SDD, Tasks 1–7, PR [#931](https://github.com/cairn-ehr/cairn-ehr/pull/931)) · 2026-11-03 (slice 3a plan, same PR) · 2026-11-03 (slice 3 brainstorm → design + review, PR [#930](https://github.com/cairn-ehr/cairn-ehr/pull/930)) · 2026-11-02 (#926 + #924 + #919 decided, ADR-0086, PR [#928](https://github.com/cairn-ehr/cairn-ehr/pull/928)) · 2026-11-01 (slice 2: brainstorm → design → plan → SDD, PRs [#917](https://github.com/cairn-ehr/cairn-ehr/pull/917) + [#920](https://github.com/cairn-ehr/cairn-ehr/pull/920)) · 2026-11-01 (#898 + #899, PR [#911](https://github.com/cairn-ehr/cairn-ehr/pull/911)) · 2026-10-31/11-01 (GUI milestone: sign-in design, plan, slice 1 built, PR [#897](https://github.com/cairn-ehr/cairn-ehr/pull/897)) · 2026-10-31 (#893, PR [#895](https://github.com/cairn-ehr/cairn-ehr/pull/895)) · 2026-10-30 (#875 option 2, PR [#892](https://github.com/cairn-ehr/cairn-ehr/pull/892)) · 2026-10-30 (#810, PR [#890](https://github.com/cairn-ehr/cairn-ehr/pull/890)) · 2026-10-30 (#882 + #885, PR [#887](https://github.com/cairn-ehr/cairn-ehr/pull/887)) · 2026-10-29 (#878, PR [#886](https://github.com/cairn-ehr/cairn-ehr/pull/886)) · 2026-10-29 (#880 + #881, PR [#883](https://github.com/cairn-ehr/cairn-ehr/pull/883)) · 2026-10-29 (#876, PR [#879](https://github.com/cairn-ehr/cairn-ehr/pull/879)) · 2026-10-28 (#875 option 1, PR [#877](https://github.com/cairn-ehr/cairn-ehr/pull/877)) · 2026-10-28 (#807, PR [#874](https://github.com/cairn-ehr/cairn-ehr/pull/874)) · 2026-10-28 (#855 + #837 item 1, PR [#873](https://github.com/cairn-ehr/cairn-ehr/pull/873)) · 2026-10-27 (#869, PR [#872](https://github.com/cairn-ehr/cairn-ehr/pull/872)) · 2026-10-26 (#861, PR [#868](https://github.com/cairn-ehr/cairn-ehr/pull/868)) · 2026-10-24 (#863 + #864, PR [#865](https://github.com/cairn-ehr/cairn-ehr/pull/865)) · 2026-10-23/24 (#857 + #859, PR [#860](https://github.com/cairn-ehr/cairn-ehr/pull/860)) · 2026-10-23 (#835 + #854, PR [#856](https://github.com/cairn-ehr/cairn-ehr/pull/856)) · 2026-10-23 (#843 + #849, PR [#853](https://github.com/cairn-ehr/cairn-ehr/pull/853)) · 2026-10-23 (#848, PR [#851](https://github.com/cairn-ehr/cairn-ehr/pull/851)) · 2026-10-22 (#846, PR [#847](https://github.com/cairn-ehr/cairn-ehr/pull/847)) · 2026-10-22 (#844 + #836, PR [#845](https://github.com/cairn-ehr/cairn-ehr/pull/845)) · 2026-10-22 (#839 + #840 + #841, PR [#842](https://github.com/cairn-ehr/cairn-ehr/pull/842)) · 2026-10-21 (#832, PR [#838](https://github.com/cairn-ehr/cairn-ehr/pull/838)) · 10-21 (#803 slice A, PR [#831](https://github.com/cairn-ehr/cairn-ehr/pull/831)) · 10-20 (#828, PR [#830](https://github.com/cairn-ehr/cairn-ehr/pull/830)) · 10-20 (#826, PR [#829](https://github.com/cairn-ehr/cairn-ehr/pull/829)) · 2026-10-19 (#822 + #823, PR [#827](https://github.com/cairn-ehr/cairn-ehr/pull/827)) · 2026-10-18 (#817, PR #821) · 10-18 (#815 + #818, PR #819) · 10-18 (#814, PR #816) · 10-18 (#811, PR #813) · 10-18 (PR #805's second review, fixed) · 2026-10-17 (#797, PR [#805](https://github.com/cairn-ehr/cairn-ehr/pull/805)) · 10-17 (#799 · #800 · #801, PR [#802](https://github.com/cairn-ehr/cairn-ehr/pull/802)) · 10-17 (PR #798's second review round) · 2026-10-16 (#745 · #752 · #740, PR [#798](https://github.com/cairn-ehr/cairn-ehr/pull/798)) · 10-16 (#793 · #737, PR #795) · 10-16 (#781 · #789 · #732, PR #794) · 10-16 (R6 residuals, PR #792) · 10-15 (R7, #742 + #743 part 2, PR #788) · 10-14/15 (R6, #716 + #723, PR #777) · 10-12/13 three cloud tech-debt passes (PRs #770–#776, ADR-0080) · 10-11 DOB near miss (PR #758, ADR-0079) · 10-08/09 R5b (PR #749, ADR-0078) · 10-06/07 R5a (PR #735) · 10-05 #725 (PR #733) · 10-04/05 R4 (ADR-0076 decision 7, PR #724) ·
10-03 R3 (PR #721) and R1b (PR #717) · 09-30 R2b-2 (ADR-0077, PR #711) · 09-29 R2b-1 (PR #707) · 09-27 R2a (PR #698,
`db/055`) and R1 (ADR-0076, `db/054`, PR #688) · 09-26 #671 (ADR-0075, PR #678) · 09-23 funnel 2c (PR #674) · earlier:
ROADMAP. · **Spec:** **v0.92** (newest ADR-0090, PR #984; v0.91 ADR-0089, PR #981; v0.90 ADR-0088; v0.89 ADR-0087; v0.88 ADR-0086; v0.87 ADR-0085; v0.86 ADR-0084; v0.85 ADR-0083; #857 and #861 needed no ADR — a projection-side fix, no wire change;
[ADR-0067](spec/decisions/0067-a-restore-reads-the-clinical-plane.md) supersedes ADR-0026 decision 2's implementation
wording only) · **`SCHEMA_GENERATION`:** **69** (`db/068` + `db/069`, PR #984, which also edits db/037, db/044, db/065 and db/066 in place; `db/067`, PR #981, which also edits db/065 and db/066 in place; `db/066`, PR #966; `db/064` + `db/065`, PR #931, which also edits db/044 and db/037 in place; `db/063`, PRs #917/#920; `db/062`, PR #911; `db/061` PR #897; `db/060`, PR #860; R6 and R7 edit db/054, R7 db/056, in place) · **Phase:** architecture
complete; **first production clinical surface RUNNING** — `cairn-node` plus a Tauri 2 window: the funnel front door (one
row per person) onto a medication chart that reads linked charts as one; a per-node matcher worker proposes duplicates,
a banner on the chart shows each one for a human to judge, a tray at the front door lists them all, and a
doubted link can be confirmed from the window; clinicians sign in per act and write, autosave and sign a progress note,
several writers to a draft, an untouched draft goes in unsigned by itself, and a clinician can discard their own draft, leaving a signed record of it.

**Built so far** (orientation only; ROADMAP + ADR log + git carry the detail): demographics 1–5 · §5.2 advisory Python
matcher · §5.7 identity core C1–C5 (C5+ `reattribute` waited on a clinical-note surface, which 3a now gives it) · §5.4 John-Doe (§5.12
push-alert open) · §5.3/§5.8 funnel (ADR-0061; precedence #345 at db/005 step 8b; ranking ADR-0075; one row per
PERSON since R3, PR #721) · `clinical.medication` 1–6b under born-sealed bodies (ADR-0052) + per-write human authorship
(ADR-0053 — grading half-live until #245) · §5.9 stream through its read surface · the text progress note `clinical.note.asserted` + its draft store (ADR-0087, slice 3a), per-writer parts (ADR-0088) the quarantine commit + `Asserted` (ADR-0089) and the audited discard (ADR-0090) · med-list node tier (read + whole-list
sign-off over a linked set, R1) · human link/unlink judgements (`chart_link`, attested-first `patient_link`, R2a) · the
compare-and-link panel (R2b-1, PR #707) + unlink from a record (R2b-2, PR #711) + a doubted set withholds every line
not on the opened chart (R1b, PR #717) · the commit-time duplicate check (`db/056`, `cairn-matcher watch`, `cairn-node
duplicate-check`; R4, PR #724; per-chart range blocking anchored, #725) · the possible-duplicate banner + "Different
people" (`db/057`; R5a, PR #735) + the front door's possible-duplicate tray (R5b, PR #749) · confirming a doubted link
from the window and a doubted row at the front door (R6, PR #777) · a "running" line on a checked chart and a note on an unverified link (R7, PR #788) · generic reprojection (ADR-0057; ADR-0070) ·
ADR-0056 admit-uninterpreted floor · **the L3 reference UI** `cairn-gui/` (standalone workspace, one-way GUI → crates;
`cairn-gui-tauri`, the iced shell FAILED a11y, spike 0004; plain JS, no npm); pane/routing/freshness state machine
tested but **not wired**.

---

## Recent sessions — what to carry forward

ROADMAP carries the per-slice narrative and every open issue number; this keeps only lessons that generalise (each
slice's own rules are its block in `developers/durable-rules.md`). **Every session since 10-16 found HANDOVER stale on
arrival** (a merged PR still "awaiting") — house rule 8's check is not optional.

### 2026-11-04 → 11-05 — sign-in slice 3b-2b (draft PR [#984](https://github.com/cairn-ehr/cairn-ehr/pull/984)): plan → SDD, 7 tasks

Each task reviewed and re-reviewed (ledger: `.superpowers/sdd/2026-11-04-sign-in-slice-3b-2b-audited-discard/`,
gitignored); the binding rules are ADR-0090 and the 3b-2b durable-rules block.
- **⇒ A plan's COPY borrowed from a sibling gesture is a claim about THIS gesture, and was false three times.** The
  plan reused Sign's words for the discard: the save's fork words ("signed in another window"), the refusals
  ("nothing was signed", and "says when it goes in by itself", false for a stuck draft), and the who's-here picker's
  "Whether your note was signed". Each review caught one. Walk every borrowed sentence through every state of the
  NEW gesture before reusing it; Sign's own leftover is #986.
- **⇒ A signature proves "this key signed A discard", not "this one".** The Task 3 review found a colleague's captured,
  genuinely signed discard could be replayed by a raw-SQL client; the fix binds the event to an instant the door mints
  under the lock (`t_effective = spent_at`). What the floor cannot see (deleted rows) is a named residual, #987.

### 2026-11-01 → 11-04 — sign-in slices 2 (PRs #917 + #920), 3a (PR [#931](https://github.com/cairn-ehr/cairn-ehr/pull/931)), 3b-1 (PR [#966](https://github.com/cairn-ehr/cairn-ehr/pull/966)), 3b-2a (PR [#981](https://github.com/cairn-ehr/cairn-ehr/pull/981))

Condensed 2026-11-04; each was plan → SDD with every task reviewed and re-reviewed, its rulings recorded in its ADR
(0085, 0087, 0088, 0089) and its durable-rules block. Slice 2's two real cases (a junior's password sent to an inbox that
needs it; a locum's Friday-night start) turned "re-key by token" into in-person enrolment.
- **⇒ A plan's own wording can be the defect** ("empty = `btrim(…) <> ''`": the floor's blank test is
  `cairn_twin_is_present`, the only one that agrees under every collation). **"Own part" by key, not by text.**
- **⇒ An escape is a wire decision, and so is the twin of an escaped text**; a consuming `(^|[^\\])` prefix skips the
  next match (use a lookbehind). **Refusing a newer profile at the remote door wedges sync**; admitting it unchecked is
  safe only if nothing shows it, so every reader reads the projection.
- **⇒ "Spent by a note" must mean "by THIS transaction's note"** (`xmin`), and a savepoint around the submit then
  breaks every sign.
- **⇒ "Is this key current" has two meanings**: admission asks *now*; re-judging a stored event asks *does it still
  vouch*. **A test's fixture can make its assertions unreachable** (a superuser approval auto-confirms, so the worklist
  was always empty). **Fact-checking the ADR is a code review** (it found four window gaps the task reviews passed).
- **⇒ A door's helpers live in the door's own file, ahead of it**; `CREATE OR REPLACE` cannot change a return shape.
  **Ordering assumptions between tasks break the build first**: run a door split across tasks as one dispatch.
- **⇒ Walk every path that LEAVES the editor**, and every sentence the window says through every branch (the "moved"
  sentence was untrue for an edited saved text; the picker's "sign it again" is impossible after "already signed",
  #970). WebKit is still owed (RUNBOOK §18–§20).
- **3b-2a (PR #981, 2026-11-04):** a failure the user can act on must reach the user, not only stderr (a dropped
  `let _ =` made a failing commit read "tries again" forever); a new file under `cairn-gui/*/src`, test fixtures
  included, is read by the ROOT tree's source guards (four tripped: run every `production_rust_files` guard DB-free,
  with its own target dir); a displayed time is a claim, computed from the door's own input (`cairn_note_drafts_due`);
  walk an offered remedy through the refusal it meets ("sign it" a stuck draft is refused too); a prescribed helper
  can be the defect (one connection hold per draft); "human" is the registry's actor KIND, not "nameable".
- **Mechanics:** a harness background job can be SIGTERMed (exit 143) — run the 2 h gate detached (`nohup … & disown`)
  in a worktree pinned to the PR head, with a done-marker; a second scratch DB set (`cairn_sdd/2/3` on :5532,
  `cairn_pgx` + the scratch marker) lets DB work proceed beside the gate without the `cairn_test` DDL deadlock.

### 2026-10-31 → 11-01 — sign-in slice 1 (PR [#897](https://github.com/cairn-ehr/cairn-ehr/pull/897)), #893 (PR [#895](https://github.com/cairn-ehr/cairn-ehr/pull/895)), #898 + #899 (PR [#911](https://github.com/cairn-ehr/cairn-ehr/pull/911))

Condensed 2026-11-03; each was brainstorm → design → plan → SDD, every task reviewed. Slice 1 filed #898–#902; #911
filed #909, #910, #912. Durable-rules carries each block.
- **⇒ A transactional counter is no throttle against a raw-SQL client** (`BEGIN; guess; ROLLBACK;` undid the count):
  count in state a rollback cannot undo (sequences, with `setval`, not `nextval`, whose WAL pre-log replays phantom
  failures), BEFORE revealing anything. A clock that stepped BACK must allow, not lock out. An Argon2-latency gap is a
  race window (a generation counter stamped at the wipe); an empty passphrase must never reach a counting door.
- **⇒ Check WHICH role the client actually connects as before promising a role-based protection** (the window was
  the owner, #898). **`has_*_privilege` and `pg_has_role(…, 'USAGE')` see only INHERITED power**: judge every fact
  over every role the login is a MEMBER of, and judge `session_user`, never `current_user` (`options='-c role=…'`).
  **Table ownership is not the only owner power:** a definer with `search_path = public, pg_temp` calling helpers by
  bare name runs the code of whoever owns or can `CREATE` in `public` (db/062 revokes `CREATE` from `PUBLIC`).
- **⇒ An issue's suggested fix is a claim:** `SET ROLE` after the schema load closes nothing (`RESET ROLE`). An owner
  verb taking a role name must refuse an existing role that is not exactly what it makes. `pg_hba_file_rules` is the
  FILE, not the loaded rules. Several dependent DDL statements go in ONE `batch_execute`.
- **⇒ A permission walk on EMPTY data proves nothing** — drive each call to its deep path. **Removing a call removes
  its side effects** (the window's schema load was the only routine deferred-event runner, #912). A rule that
  normalises in SQL must also BUILD the derived value in SQL. Visibility: a strip inside a hidden parent is invisible.
- **⇒ An aborted tokio-postgres task gives every pending query the same `Error::closed()`** (#893): the cause is a
  context layer added by whoever holds BOTH the guard and the error. "Every call site" is not "every error path".

### 2026-10-18 → 10-30 — the W, interrupted commits, the lost connection (PRs #813–#890)

Condensed 2026-11-03; ROADMAP has each PR, durable-rules each slice's rules. Interrupted commits #813–#830 (#811,
#814, #815, #820, #826, #828), the W #831 + #838 (#803, #832), #842 (#841), #847 (#846), #851 (#848), #853 (#843 +
#849, ADR-0082), #856 (#835 + #854), #860 (#857 + #859), #865 (#863 + #864; filed **#866**, **#867**), #868 (#861;
filed #869, **#870**, **#871**), then #874 (#807), #877 + #892 (#875), #879 (#876), #883 (#880 + #881), #886 (#878),
#887 (#882 + #885), #890 (#810), #873 (#855 + #837 item 1). #892's brainstorm filed **#891**. Not done then: #812 items
7–10, #814; nothing reads Postgres notices (#585). Decisions still owed: #870 · #871 · #866 · #867 · #825 · #850.
- **⇒ A timeout around a tokio-postgres call does not free the connection** — abort the connection TASK; aborting the
  client never tells the server (`pg_terminate_backend` the old backend first); a new timeout around a call with its
  own must be LONGER than it. A deferred trigger that sleeps makes COMMIT the slow statement.
- **⇒ A new meaning gets a new field, not a status-dependent reading of an old one**; widen a set-returning read with
  a sibling predicate (`CREATE OR REPLACE` refuses a return-type change). A new projection beside a one-winner one: a
  second apply fn, a generation bump, the shred delete, BOTH registry counts, the fixture truncate lists.
- **⇒ Walk every remedy, absence or future-event sentence through every state it is shown in**, and every operator
  procedure against the state it needs. Routing a new cause into an existing variant reaches every renderer of it; a
  `From` impl makes `?` compile where it is wrong. Say what the USER must do.
- **⇒ "Never over-erase" is half a rule** — a scrub of a winner-takes-all projection owes a re-fold; never SKIP a
  locked sibling. **An outcome that MAY have written settles like a success**; only an ERROR answer proves a
  rollback. A floor `P0001` carries no scope and may be stale: re-ask it, never parse it (#855). The logical clock is
  not a wall clock (#803). Two humans with the same determinants are ONE actor (#152).
- **⇒ Check a mutation APPLIED before trusting the green after it.** A real mouse finds what `element.click()` cannot;
  a root crate add moves `cairn-gui/Cargo.lock`; a moved doc link breaks only in `cargo doc`.
- **Smaller, still true:** an `IF NOT EXISTS` index never picks up an edited predicate; a flag written at arrival
  sees only what had arrived (read at list time); a flag carried by one async call dies with it (make it window
  state); state touched only under a lock lives INSIDE it (`MutexGuard::map`); a fallback in the floor bypasses the
  rule the client follows; an ADR consequence a later fix closes gets a dated erratum (ADR-0082 E1).

### 2026-10-04 → 10-17 — R4–R7 (PRs #724, #733, #735, #749, #777, #788), cloud passes (PRs #770–#776, #779), #797 (PR [#805](https://github.com/cairn-ehr/cairn-ehr/pull/805))

Condensed 2026-10-31; ROADMAP has each slice. #797: brainstorm → plan → SDD; the maintainer's rule (durable-rules):
*tell the clinician only what changed from ANOTHER source since they took the chart*; #803/#804 decided there. Also
#745 · #752 · #740 (PR #798), #799 · #800 · #801 (PR #802), PR #792 (R6 residuals #787 · #786 · #783 · #782), splits
#781 · #789 · #732 · #793 · #712 item 3 · #737 · #796 (PRs #794, #795); R6 = #716 + #723; R7 = #742 + #743 part 2.
Filed then: #781 · #782 · #783 · #784 · #785 · #786 · #787 · #789 · #790 · #791 · #755 · #756 · #757 · #778; commented #718 · #738; decisions left: #649 · #613 · #628 · #780
(`casefold()`), Turkish dotless `ı` (the ADR-0014 profile's job); #427 is lower than it reads (#424); #220 gained a test; #626's `do_pull` non-P0001 arm open.
- **⇒ Route a cause by TYPE end to end** (`RefusalScope::Stale` → `DataError::Stale` → `Retry::AfterReread`); a baseline
  that must survive overlapping reads is window STATE; rule on the evidence, not on who proposed it; pin "after apply,
  before commit" on BOTH sides; walk the OLD code with the same script (a walk passing only on the fix proves the rig);
  webview JS is pinned without a JS runner (`source_guard::js_fn` / `js_const`), on the exact expression.
- **⇒ A remedy that names a command is a claim: run it, name the LIGHTEST** (#787); keep one in-memory truth (#786);
  prove a pure move by the sorted test LIST; VoiceOver reads "—" as a pause. A title is a claim too (ADR-0078); count
  steps against the as-built window (#754); read the design's filter against its issue (#700); diagnose before
  choosing a fix (#725, `EXPLAIN ANALYZE`).
- **⇒ #726: a connect replays the schema only when it changed** (ADR-0080; tests call `db::force_schema_replay`). Also:
  a case-mapping change on one side of a subset invariant breaks it (#775); COMMITTED IS NOT VOUCHED (#556); a
  judgement carries what was compared (#708); an unknown outcome is its own ROOT marker (#713); a per-row lock cannot
  guard a whole-log scan (#773); an issue's fix can be wrong about MVCC (#560) or predate a guard (#608 vs #633); a
  count is not a set (#553); a lenient arm can be worse than silent (#768, #606).
- **⇒ ROLLBACK HAZARD:** with `SCHEMA_GENERATION` unchanged, an OLDER binary on a database this branch loaded fails
  "cannot change return type" on db/056 — `DROP FUNCTION cairn_duplicate_check_status();` before switching back.
- **Cloud rig** (runs the FULL DB gate): micromamba + conda-forge `postgresql=18`, `cargo install cargo-pgrx --version
  =0.18.1 --locked`, `cargo pgrx init --pg18 <conda pg_config>` with an `xlocale.h` shim in `CFLAGS` and
  `BINDGEN_EXTRA_CLANG_ARGS`, `initdb` as `postgres`, `CREATE ROLE root SUPERUSER`; never pre-create `cairn_pgx` in
  `template1`; `apt-get install libwebkit2gtk-4.1-dev libgtk-3-dev libsoup-3.0-dev`; `target/` ~23 GB; a new db/tests
  mirror needs `\ir _scratch_database_guard.sql`. `cargo doc -D warnings` reads `<date>` as an unclosed HTML tag; a
  test assert must never echo identity-built text (CodeQL failed #795 after merge).
- **Smaller:** draw unless something newer is DRAWN, say a failure only if the newest read STARTED; a test whose two
  orientations coincide pins neither; a callee's `rollback()` undoes a test's `SET ROLE`; stub payloads are dumped
  from Rust, never hand-written; a CI job with a different dependency set cannot be gated locally.

### 2026-08-20 → 10-03 — restore, node plane, doors, search, funnel UI 2a–2c, #671, repair path R1–R3

Narrative: ROADMAP; its lessons now live as durable-rules traps 7–18 and in memory ("SDD evidence + ADR fact-check",
"SET ROLE fault-injection seam", "a gate is evidence only for the tree it ran against"). In short: a remedy or count
sentence is a safety claim, walked through every case; "inside the transaction" is not "serialized" (take the lock
every writer holds, CARNLK); a test that cannot see its own predicate proves nothing, so mutate to check; a new ADR
needs its `mkdocs.yml` nav line. Its issues still open: #335 · #583 · #586 · #598 · #620 · #624 · #656 · #657 · #658 ·
#703 · #712 · #718 · #719 · #720 · #722.

**⇒ OPEN ISSUES OLDER SESSIONS OPENED, INDEXED** (never drop an open number while condensing):

[#288](https://github.com/cairn-ehr/cairn-ehr/issues/288) · [#327](https://github.com/cairn-ehr/cairn-ehr/issues/327) · [#394](https://github.com/cairn-ehr/cairn-ehr/issues/394) · [#402](https://github.com/cairn-ehr/cairn-ehr/issues/402) · [#406](https://github.com/cairn-ehr/cairn-ehr/issues/406) · [#407](https://github.com/cairn-ehr/cairn-ehr/issues/407) · [#408](https://github.com/cairn-ehr/cairn-ehr/issues/408) · [#409](https://github.com/cairn-ehr/cairn-ehr/issues/409) · [#413](https://github.com/cairn-ehr/cairn-ehr/issues/413) · [#420](https://github.com/cairn-ehr/cairn-ehr/issues/420) · [#422](https://github.com/cairn-ehr/cairn-ehr/issues/422) · [#428](https://github.com/cairn-ehr/cairn-ehr/issues/428) · [#430](https://github.com/cairn-ehr/cairn-ehr/issues/430) · [#431](https://github.com/cairn-ehr/cairn-ehr/issues/431) · [#447](https://github.com/cairn-ehr/cairn-ehr/issues/447) · [#458](https://github.com/cairn-ehr/cairn-ehr/issues/458) · [#463](https://github.com/cairn-ehr/cairn-ehr/issues/463) · [#464](https://github.com/cairn-ehr/cairn-ehr/issues/464) · [#470](https://github.com/cairn-ehr/cairn-ehr/issues/470) · [#483](https://github.com/cairn-ehr/cairn-ehr/issues/483) · [#484](https://github.com/cairn-ehr/cairn-ehr/issues/484) · [#485](https://github.com/cairn-ehr/cairn-ehr/issues/485) · [#487](https://github.com/cairn-ehr/cairn-ehr/issues/487) · [#488](https://github.com/cairn-ehr/cairn-ehr/issues/488) · [#490](https://github.com/cairn-ehr/cairn-ehr/issues/490) · [#491](https://github.com/cairn-ehr/cairn-ehr/issues/491) · [#492](https://github.com/cairn-ehr/cairn-ehr/issues/492) · [#494](https://github.com/cairn-ehr/cairn-ehr/issues/494) · [#504](https://github.com/cairn-ehr/cairn-ehr/issues/504) · [#505](https://github.com/cairn-ehr/cairn-ehr/issues/505) · [#506](https://github.com/cairn-ehr/cairn-ehr/issues/506) · [#507](https://github.com/cairn-ehr/cairn-ehr/issues/507) · [#508](https://github.com/cairn-ehr/cairn-ehr/issues/508) · [#509](https://github.com/cairn-ehr/cairn-ehr/issues/509) · [#513](https://github.com/cairn-ehr/cairn-ehr/issues/513) · [#518](https://github.com/cairn-ehr/cairn-ehr/issues/518) · [#521](https://github.com/cairn-ehr/cairn-ehr/issues/521) · [#522](https://github.com/cairn-ehr/cairn-ehr/issues/522) · [#529](https://github.com/cairn-ehr/cairn-ehr/issues/529) · [#530](https://github.com/cairn-ehr/cairn-ehr/issues/530) · [#543](https://github.com/cairn-ehr/cairn-ehr/issues/543) · [#545](https://github.com/cairn-ehr/cairn-ehr/issues/545) · [#557](https://github.com/cairn-ehr/cairn-ehr/issues/557) · [#558](https://github.com/cairn-ehr/cairn-ehr/issues/558) · [#559](https://github.com/cairn-ehr/cairn-ehr/issues/559) · [#560](https://github.com/cairn-ehr/cairn-ehr/issues/560) · [#561](https://github.com/cairn-ehr/cairn-ehr/issues/561)

---

## Read these first (the durable state)

CLAUDE.md carries the document hierarchy in full; this adds only what it does not.
**`docs/developers/durable-rules.md`** — the per-slice rules and the numbered trap list every session must hold (moved out
of this file in R7; a new rule or trap goes there, never here). **`docs/spikes/`** — 0001 (walking
skeleton — Bet A ✓ → ADR-0015; Bet B ✓ twice); 0002 (advisory-actor, C1–C5 ✓ → ADR-0029/0030); 0003 (Postgres on
Android, G0–G3 ✓); 0004 (iced UI — FAIL on a11y → Tauri 2). **`docs/case-studies/0001`**: 16 GP-software failure modes,
all absorbed, **0 new architecture**. **`docs/ecosystem/`** 0001, 0003 · **`docs/principles/`** — mission/governance.
Code workspace: `/crates` (`cairn-db-keepalive`, `cairn-event`, `cairn-keystore`, `cairn-medium`, `cairn-wire`, `cairn-sync`, `cairn-node`,
`cairn-medication-view`, `cairn-patient-search`), `/extensions` (`cairn_pgx`), `/db`, `/cairn-gui` (separate workspace);
`poc/` is frozen historical spikes. **Where the build is:**

- **First federating node** (ADR-0017) — `cairn-node`: Ed25519 keystore, pairing/`peers`/`unpeer`, mTLS pinned to the
  trust set, set-union `node_event` sync, `db/007`'s doors with a deny-all admission gate, genesis-stable `node_id`.
  Custody travels, the clinical event log reaches the medium, and a restore reads it back (ADR-0067); optional escrow
  rungs (Shamir/QR/TPM) remain. **Dual-identifier discipline** (ADR-0031): the canonical plane (UUIDv7 + multihash) is
  the only identifier on the wire; the projection plane may intern node-local `bigint` surrogates (`db/008`).
- **Test rig:** DB-gated tests need local PG18 + `cairn_pgx`, self-serializing via a Postgres advisory lock
  (`db::test_serial_guard`). **Advisory locks are scoped PER DATABASE** (#467; ~124 comments still say otherwise,
  **#476**), so every caller takes the guard against `CAIRN_TEST_PG`. The tech-debt loop (`/techdebt-loop`,
  `/techdebt-next`) is STOPPED (above).

---

## Open threads — pick one (today's-work menu)

**Desk-doable now (no external dependency):**
- **⇒ Eval 0005 follow-ups (CockroachDB lessons), accepted 2026-11-03 — epic [#933](https://github.com/cairn-ehr/cairn-ehr/issues/933).**
  Order: the **syncnemesis** convergence harness first ([#934](https://github.com/cairn-ehr/cairn-ehr/issues/934),
  plan `superpowers/plans/2026-11-03-syncnemesis-harness.md`, slices S0–S6), then **F1**
  ([#935](https://github.com/cairn-ehr/cairn-ehr/issues/935)). F1: the HLC drift ceiling trusts the local clock, so a
  node running behind refuses honest node-plane events, revocations included, and mints corrections below their
  targets. It also aborts a DR restore onto a box whose clock is behind (db/009). And once a clock regresses
  more than 24 h below the node's own HLC, the node refuses its *own* clinical writes (db/005). That last one
  surfaced in the PR #932 review, and #935 now carries it as consequence 4, with tests (d)/(e) and design
  option D. The eval's peer-median remedy does **not** cover it, since an offline node has no peers to ask, so
  #935's option D (a local-door rule) is open. Its reproduction comes free in harness slices S4/S5.

  **Decision owed before S4 step 2 (plan D7, options A/B/C; B recommended):** where the per-node clock skew
  lives. A redefined `cairn_wall_ms()` cannot survive, because `cairn-sync` replays every migration on every
  connect. Nor does extending the #726 replay-skip to cairn-sync fix it: `node_schema` holds one stamp naming
  one loader, so cairn-node and cairn-sync would hand it back and forth and both keep replaying. ADR threads
  #937–#939; engineering #936, #940–#943.
- **⇒ The repair path: the open repair-path issues** — see ⇒ NEXT.
- **⇒ DR — closed and rehearsable end to end.** Not there, though a reader expects it: **2d does NOT drive
  `cairn-sync`'s puller through `MediumTransport`**; **the per-peer quarantine quota does not apply to a
  restore-originated pen** (`restore_pen_is_uncapped.rs`). Filed by the chain, beyond ⇒ NEXT's node-plane list: #525 ·
  #541 (no CI job compiles `cairn_pgx`'s `pg_test`) · #531/#329 (decompose `cairn-sync/src/main.rs` — maintainer picks
  one) · #532 · #534 · #535 · #537 · #538 · #624–#626. (Ranges absorb issues that leave them: re-check against GitHub.)
- **§5.9 parts C/D** (#232) — above.
- **`clinical.medication` — slices 1–6b DONE, read over a linked set since R1.** Next: #690 (reconciling across linked
  charts), the **drugref term→anchor lookup**, fuzzy/automatic reconciliation + a Tier-A dictionary, structured
  sig/frequency, correcting a dose event's effective date. Cross-cutting debt **#185**. Spine: `db/031`–`db/035`,
  `db/041`, `db/042`, `db/054`. **No allergy stream exists yet** — the repair path's "active medications" becomes
  "allergies and active medications" when one does.
- **Demographics / matcher / identity — next slices** (`db/010`–`db/030` + `cairn-event::demographics`). B3-driven: gold
  set, locale packs, hub-tier duplicate sweep (R4 is its per-node sibling), proposal retraction. Identity: C5+
  `reattribute` (its clinical-note surface exists since 3a); §5.12 push-alert. Deferred **#168**, **#287**; rest in ROADMAP.
- **⇒ Test env — `scripts/run-db-gated-tests.sh` is the ONE command for the local gate**, the only one catching all
  three demonstrated hiding modes (fail-fast · a piped exit status · a cross-crate suite `-p <crate>` never builds).
  **The CLUSTER is discovered** (`scripts/pg-target.sh`, PG ≥ 18; set `PGPORT` to name one). **Cost depends on what
  changed — measure, never budget from a figure here.** Without the three env strings the DB suites self-skip, so a
  bare run FAILS unless `CAIRN_ALLOW_DB_SKIP=1` is declared (#450). Mirrors are DESTRUCTIVE, refusing any DB lacking the
  `cairn_scratch_database` marker (#169). Matcher: `cd matcher && CAIRN_TEST_PG=… uv run --extra pipeline pytest` (uv,
  never pip; gap **#314**). `clinical_pull` flakes under a full run: `--test-threads=2`.
- **Clinical case-mining** — historically the highest-signal generative mode. Bring a real ED/hospital failure mode;
  record in [`docs/case-studies/`](case-studies/README.md). Open from Case 0001: **① re-affirmation-without-change
  currency** (#163); **② open-loop/obligation**, a projection surfaced by salience not a modal; **③ impossible-vs-uncertain**
  for the in-DB floor.
- **Landing-page polish** — a non-developer page for the generated site (`web/`).

**Blocked on hardware / external access:** **Bet B — Pi compute-cost run**
([Spike 0001 §9](spikes/0001-walking-skeleton-wan-sync-and-pi-cost.md)): PASS twice; fold the un-caveated B4 number into
ADR-0015 to drop "provisional"; **#272** (reproject bench on the Pi). **easyGP session** — port
[ADR-0020](spec/decisions/0020-active-write-thin-encounters-and-the-delete-vs-erase-distinction.md)'s deferred items with
live schema access (`rx!`/`tx!` parser + state machine; formulation/drug source + forced-manual rule table; prefetch
warming daemon); pre-read `scratch/ui-sketches/easygp-prefetch-notes.md`. **Byte-tier throughput lever** — connection
reuse / persistent streaming instead of one TCP connection per slice.

---

## Parked · Working context

- **Parked (don't re-litigate without new reason):** legal entity & jurisdiction — deferred until momentum/funding
  geography is clearer; trademark registration — principle recorded, instrument deferred.
- **CLAUDE.md carries the working context in full and is loaded every session.** Canonical docs win.
- **Governance done** ([GOVERNANCE.md](principles/GOVERNANCE.md) + root `CONTRIBUTING.md`): AGPL-3.0
  inbound=outbound, DCO, **no CLA**; mission as tie-breaker. Names/domains/packages secured.
