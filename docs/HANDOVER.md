# HANDOVER — Cairn

## ⇒ NEXT

> [!IMPORTANT]
> **⇒ THE NEXT GOAL (maintainer, 2026-10-31): a GUI milestone** — register a clinician as a user, log in, search /
> select / register a patient, enter demographics, write a progress note, record an allergy, chart a medication,
> sign everything off, log out. Four decisions taken in conversation: (1) **a superuser role assigns new users**, at
> least one created at installation; (2) **a key vault holds the clinicians' signing keys** (login = "choose who you
> are + passphrase") — both built, ADR-0083; (3) **a draft auto-committed after the quarantine is RECORDED, NOT
> ATTESTED** — built, [ADR-0089](spec/decisions/0089-the-quarantine-commit-and-the-asserted-grade.md); (4) **"prescribe"
> in this milestone means CHARTED** (printed and electronic prescriptions come much later) — recorded in
> [ADR-0099](spec/decisions/0099-charting-a-medication.md) decision 1.
>
> **Its sign-in / authorship part:** design
> [`superpowers/specs/2026-10-31-gui-sign-in-and-authorship-design.md`](superpowers/specs/2026-10-31-gui-sign-in-and-authorship-design.md)
> (builds ADR-0008 §5.11; slices 1 sign-in → 2 re-key (#247, #664 first) → 3 drafts + `Asserted` grade + its display →
> 4 emergency key → 5 autonomous terminal nodes that sync credentials with specified nodes).
> - **Slices 1 → 3b-2a MERGED 2026-10-31 → 11-04** (ROADMAP and each ADR have the detail; open issues by slice):
>   **1** (PR #897, [ADR-0083](spec/decisions/0083-the-human-key-vault.md), `db/061`, `cairn_pgx` 0.4.0: the key vault,
>   per-act signing, the strip, the terminal lock, "Enrol staff", one `EventSigner` seam). **2** (PRs #917 + #920,
>   [ADR-0085](spec/decisions/0085-in-person-enrolment-and-re-key.md), `db/063`: in-person enrol / re-key,
>   *unconfirmed* until a superuser judges; `rotate-key` keeps the actor; #926 in
>   [ADR-0086](spec/decisions/0086-who-may-act-after-a-repudiation.md), #919 + #924 accepted). **⚠️ Ruling R4:** remote
>   apply admits any key in `actor_key_vouches`, so a leaked, rotated-away, UNrepudiated key could inject events via a
>   peer or medium. **⚠️ A local-state export by this build will not decode on an older build.** Open: **#918** ·
>   **#921** (a decision) · **#922** · **#923** · **#925** · **#927** · **#929**; RUNBOOK §17. **3a** (PR #931,
>   [ADR-0087](spec/decisions/0087-the-first-note-stream-and-its-draft.md), `db/064` + `db/065`): escape once at
>   signing, twin UNESCAPED (R4); the attachment-id grammar is wire (R5); **read notes only from `clinical_note`** (R8);
>   a spend names a note of the SAME transaction, no savepoint (R9); drafts in no backup; `note.added` stays (#364).
>   Open: **#945** (half built) · **#946** · **#947** · **#948** (ruled penned, ADR-0091) · **#949** · **#950** ·
>   **#951** · **#952** · **#953** · **#957** (a decision) · **#959** · **#960** · **#961** · **#962**; **RUNBOOK §18 HALF
>   RUN** 2026-11-03 (`results/2026-11-03-note-stream-human-pass.md`, filed **#954**, **#955**; still owed: the
>   read-only-while-signing re-run, open ≤ 1 s, the partly typed time, stated seconds, Measurement 3, keyboard +
>   VoiceOver; to file or fold into #947: a save blocked behind a lock is silent, `recording_started` is stamped when
>   the first save executes). **3b-1** (PR #966, [ADR-0088](spec/decisions/0088-whos-here-and-a-drafts-parts.md),
>   `db/066`: who's here is a claim; per-writer parts with revision rows; "Continue here" MOVES a draft, #958; Sign =
>   one event per part chained by `addendum-to`). Open: **#963** (slice 5) · **#964** · **#965** · **#967** · **#968** ·
>   **#969** · **#970** · **#971** · **#972** · **#975** · **#976** · **#977** · **#978**; #973 built; RUNBOOK §19.
>   **3b-2a** (PR #981, [ADR-0089](spec/decisions/0089-the-quarantine-commit-and-the-asserted-grade.md), `db/067`: an
>   untouched draft goes in **unsigned** after the quarantine, `cairn-node note-set-quarantine`, default 180 min; a
>   keyless named writer leaves it **stuck**; `Asserted` reads "went in unsigned"; #974 ruled not `M > N`). Filed
>   **#979** (deferred) · **#980** · #982 (built) · **#983**; commented #968; RUNBOOK §20.
> - **Slices 3b-2b → 3c-3b MERGED 2026-11-05 → 11-07** (ROADMAP and each ADR have the detail; open issues by slice):
>   **3b-2b** (PR #984, [ADR-0090](spec/decisions/0090-the-audited-discard.md), `db/068` + `db/069`: Discard draft leaves
>   a signed, sealed `clinical.note-discard.asserted`; own parts by KEY only). Open: **#985** · **#986** · **#987** ·
>   **#988** · **#989** · **#990** · **#991** · #980 still; RUNBOOK §21. **3c-1** (PR #992,
>   [ADR-0091](spec/decisions/0091-the-compound-note-and-the-addendum.md), `db/070`: "Add to this note" → `addendum-to`
>   the ROOT; one compound note). Open: **#993** (`K = 4 > N = 3`) · **#994** · **#995** · **#996** · **#998** · **#999**
>   (a decision); RUNBOOK §22. **3c-2** (PR #1000, [ADR-0092](spec/decisions/0092-the-vouch-and-the-sign-off.md),
>   `db/071`: the vouch, one sealed human-signed attestation per part, read only verified + naming the part's own bytes
>   + its own chart; "Sign off this note" AS SHOWN; the `student` role). Open: **#1001** · **#1002** (the medication
>   pane reads the strip while a note act is in flight) · **#1003** · **#1005**; #995 extended; RUNBOOK §23. **3c-3**
>   designed in [`superpowers/specs/2026-11-06-sign-in-slice-3c-3-supervision-and-pending-signatures-design.md`](superpowers/specs/2026-11-06-sign-in-slice-3c-3-supervision-and-pending-signatures-design.md);
>   **3c-3b** (PR #1006, [ADR-0093](spec/decisions/0093-a-repudiated-signature-counts-as-unsigned.md), no SQL: a
>   since-repudiated key's part counts as unsigned until the real writer re-signs, `vouch::re_signs`). Open: **#1007** ·
>   **#1008** · **#1010** · **#1011** (needs a gesture design) · **#1012**; #918's link half stays open; RUNBOOK §24.
> - **Slice 3c-3a MERGED 2026-11-07 in PR [#1013](https://github.com/cairn-ehr/cairn-ehr/pull/1013)**
>   ([ADR-0094](spec/decisions/0094-supervision-and-the-mandatory-co-sign.md), spec v0.96, `db/072`): supervision as six
>   no-key `actor_event` ops carried by restore; "— not co-signed" judged as of the part's HLC wall; a mandatory co-sign
>   cleared only by a supervisor as of the co-sign; "Co-sign as reviewed"; the assigner panel and "My supervisees"; an
>   unconfirmed re-sign counts (maintainer). Open: **#1014** (two clocks — **gates any multi-node pilot**) · **#1015** ·
>   **#1016** · **#1017** · **#1018**. Owed by a human: **RUNBOOK §25**. A local-state export carrying a supervision
>   row is one-way.
> - **Slice 3c-3c MERGED 2026-11-07 in PR [#1019](https://github.com/cairn-ehr/cairn-ehr/pull/1019)** ([ADR-0095](spec/decisions/0095-the-person-menu.md), spec **v0.97**, `db/073`,
>   `SCHEMA_GENERATION` **73**; plan
>   [`superpowers/plans/2026-11-07-sign-in-slice-3c-3c-person-menu.md`](superpowers/plans/2026-11-07-sign-in-slice-3c-3c-person-menu.md),
>   Q1–Q3 ruled as their defaults). "Waiting for you: N" on the who's-here bar for the person NAMED there: (1) their
>   drafts, parts gone in unsigned, parts signed with their since-repudiated key; (2) supervisees' parts awaiting their
>   co-sign; (3) supervisees' notes of the last 7 days to review — collapsed, never counted. Every group is the chart's
>   own predicate over the chart's own query read by keys (`notes_sql!`, `parts_by_keys`, pure `note::menu`); db/073's
>   two read doors (people + keys; drafts by writer); "Open note" re-reads and keeps the wrong-chart rule; the picker
>   shows each person's count. Open: **#1020** (two actors sharing a staff id) · **#1021** (the reads grow with all-time
>   history and serialise with pick/save/Sign — measured, 5–6× cut, then kept to what a menu could list, 2026-11-08, see ⇒ NEXT) · **#1022** (test hygiene) · **#1023** (residuals,
>   incl. `funnel.js` at 499 lines) · **#1024** (`note::menu` types: an `Option` staff id, a `MenuItem` target enum).
>   Owed by a human: **RUNBOOK §26** (also WebKit + real Tauri IPC, walked in Chromium only). The 2026-11-08 final
>   check fixed ADR-0095 (j)'s wording, made "Open note" stay on the chart when the Sign in flight did not go
>   through (closing the chart erased that outcome), worded a failed bar read, and made the open check's charts a CI-tested
>   `PersonMenu::charts()`.
> - **Allergy slices 1 + 2 MERGED 2026-11-09** (design
>   [`superpowers/specs/2026-11-08-allergy-stream-design.md`](superpowers/specs/2026-11-08-allergy-stream-design.md);
>   ROADMAP and the ADRs have the detail). **Slice 1**, PR [#1026](https://github.com/cairn-ehr/cairn-ehr/pull/1026)
>   ([ADR-0096](spec/decisions/0096-the-allergy-list.md), spec v0.98, `db/074`): the entry + "none known" streams,
>   born-sealed, **no clear `safety` signal** (#406/#401), the terse strip, a custody-less reader told a record cannot be
>   read. Open: **#1027** · **#1028** · **#1029** · **#1030** · **#1031** · **#1032** (a decision) · **#1033** (a
>   decision) · **#1034** · **#1035** · **#1036** · **#1037**; owed **RUNBOOK §27**. **Slice 2**, PR
>   [#1038](https://github.com/cairn-ehr/cairn-ehr/pull/1038) ([ADR-0097](spec/decisions/0097-allergy-overlays-and-differing-accounts.md),
>   spec v0.99, `db/075`): Correct / De-label / Entered in error as signed overlays covering only what they name, a
>   suspect overlay counts only to ADD a warning, no act on a doubted-link chart's line, differing accounts, byline key
>   markers (#1027 built). Open: **#1039** · **#1040** · **#1041** · **#1042** · **#1043** · **#1044** · **#1045** ·
>   **#1046** (a decision) · **#1047** · **#1048** · **#1049** · **#1050** · **#1051**; owed **RUNBOOK §28**.
> - **Allergy slice 3 MERGED 2026-11-10 in PR [#1052](https://github.com/cairn-ehr/cairn-ehr/pull/1052)** (plan
>   [`superpowers/plans/2026-11-09-allergy-slice-3-drugref-and-the-medication-check.md`](superpowers/plans/2026-11-09-allergy-slice-3-drugref-and-the-medication-check.md),
>   [ADR-0098](spec/decisions/0098-allergy-coding-and-the-medication-check.md), spec **v0.100**, `db/076` with db/074 +
>   db/041 + db/042 in place, `SCHEMA_GENERATION` **76**): a drug entry may be coded `drugref-moiety` or `drugref-class`
>   (a drugref ALLERGY GROUP, after drugref PR #225); `db/076` is the one file that reads drugref (four definer doors,
>   availability from the catalogue incl. `allergy_check`'s exact shape, the facts door passing `drugref.allergy_check`
>   through — drugref owns the fold); every current medication line checked on read: the loud **Allergy** tag + words on
>   a hit, a muted note for an adverse effect, a small **not checked** tag when unverified; a door error is "no drug
>   list", never a failed pane. Open: **#1053** (code the tolerated drug) · **#1054** (check at charting) · **#1055**
>   (drugref's cephalosporin rule — `no-match` is not "safe") · **#1056** (unknown reaction word ranks last) · **#1058**
>   (review minors); from the final check, **#1060** (a reconciled group with an uncoded member can read clean) ·
>   **#1061** (a mild mark hides an unverified severe allergen — ruling needed) · **#1062** (a degraded drug list is
>   never said) · **#1063** (stale tags after a failed re-read) · **#1064** (test residuals). The final check also made
>   a failed allergy read tag every current line "not checked" and restored ADR-0097's sibling rule under a shared
>   code (ADR-0098 decisions 16 and 25). **#1057** (the group crowded out of the pick, its name doubled) and **#1059** (an unreadable
>   allergy record left the list untagged) were FIXED by its final fix wave (groups first, labelled once; unread records
>   make every unmarked current line "not checked"; a visible "Coded as: …" line under the Substance field, ruling F3)
>   — both now closed on GitHub. **#1065** (filed 2026-11-10,
>   with **drugref#231**): the pick matches only drugref's display name, so *adrenaline, lignocaine, frusemide,
>   amoxycillin, acetaminophen* find no moiety — the entry signs uncoded and reads "not checked"; needs drugref to
>   persist moiety names first (drugref #227 reads the UNII official names in ingest memory only). Owed by a human: **RUNBOOK §29**
>   (copy `drugref_allergy` from :5532 — `drugref_profiles` has no groups).
> - **Charting slice 1a MERGED 2026-11-11 in PR [#1066](https://github.com/cairn-ehr/cairn-ehr/pull/1066) (gate green at `7bfbd8ef`, 4 878 tests; the signed override generalised to `warning_override {reason, against: [{source, ref}]}` before merge); batch charting (#1072) fully ruled, designed next** (branch
>   `design/charting-a-medication`; design [`superpowers/specs/2026-11-10-charting-a-medication-design.md`](superpowers/specs/2026-11-10-charting-a-medication-design.md),
>   plan [`superpowers/plans/2026-11-11-charting-slice-1a-the-line-and-the-check.md`](superpowers/plans/2026-11-11-charting-slice-1a-the-line-and-the-check.md),
>   [ADR-0099](spec/decisions/0099-charting-a-medication.md), spec **v0.101**, `db/077` + `db/076` §5 with db/031 + db/037 +
>   db/044 in place, `SCHEMA_GENERATION` **77**). *Start medication* /
>   *Already taking* open one typed line (`amox 500mg tds`); five optional wire keys (`charted_as`, `route`,
>   `substance.strength`, `substance.components`, `warning_override` — general `{reason, against: [{source, ref}]}`, `allergy` the only source today) with their own projection `medication_charting`;
>   the drugs-only pick `cairn_drug_medication_search`, ranked by this site's use; the directions read, never guessed;
>   the check at charting, **re-run at Sign** on a fresh read, and a *Start* line with a mark needs a typed reason
>   (requiring one is a window rule, never a floor rule); a combination emits no safety claim; the list's cell is
>   `dose — sig`. **Final-check fixes (2026-11-11):**
>   - the dose is certain only when every unread word is a duration or on the `HARMLESS` allow-list (`half`, `½`,
>     `per kg`, `max` and `up to` no longer sign a dose), and two strengths read neither;
>   - Sign refuses `CHECK_DID_NOT_RUN` rather than dropping a typed reason when its own re-check could not run;
>   - a failed combinations read leaves the list "not checked";
>   - a blank pick display is refused before the key;
>   - a failed read-back shows the directions as not read. Filed: **#1067** (the coding overlay cannot code a combination) · **#1068** (no list shows the charting
>   fields) · **#1069** (a stat line stays current until slice 2) · **#1070** (the pick's accessibility) · **#1071**
>   (an override's `against` names one record per mark) · **#1072** (*Already taking*: one Sign per line, an `M > N`
>   candidate — **a maintainer decision**) · **#1073** (the typed brand is never shown beside the generic;
>   `shown_name` built, unused) · **#1074** (CI flake: schema-replay `40P01` between test binaries despite
>   `test_serial_guard` — a red floor job with that signature is it) · **#1075** (charting authoring rules
>   enforced only after the passphrase; type hardening) · **#1076** (a failed drug search reads "no drug list") ·
>   **#1077** (test gaps, incl. an unconfirmed brand-allergy-after-Sign case); commented #1054 (built without the de-label offer). Owed by a human: **RUNBOOK §30**.
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
> **Merged 2026-10-18 → 10-31** (PRs #802 → #895; ROADMAP has each, durable-rules their blocks; ADR-0081, ADR-0082).
> Still open from them: **#858** · **#850** · **#837** · **#825** · **#185** · **#870** · **#871** · **#866** ·
> **#867** · **#891** (brainstorm first) · **#894** (satellite rig) · **#862** (`run-db-gated-tests.sh` with
> `CAIRN_TEST_PG*` but no `PGPORT` runs its SQL step against
> :5432 — pass `PGHOST=127.0.0.1 PGPORT=5532`). The repair path (#679 · #680 · #681)
> is built through R7. Design `docs/superpowers/specs/2026-09-27-duplicate-repair-path-679-680-681-design.md`; R4's
> runbook `docs/developers/running-the-duplicate-check.md`; human passes RUNBOOK §11–§14. spec **v0.84** (newest
> ADR-0082). DOB-near-miss follow-ups: #759 · #760 · #762 · #764 · #765 · #766. Trap: a new blocking key can generate
> a pair an older test asserts absent — fix the fixture date, never the assertion.
>
> **⇒ NEXT, in order:**
> 0. Check `gh pr list` before trusting this list (house rule 8).
> 1. **Slice 3 is complete** (3a → 3c-3c, the bullets above). **#1021 measured, MERGED 2026-11-08 in PR #1025**
>    (figures `crates/cairn-node/results/2026-11-08-person-menu-reads-macos-m3max.md`: per-key fact lookups 5–6× faster,
>    then a lossless keep rule — **386 ms at a ward-year, 1.13 s at 100 000**); #1021 stays open for the obligations
>    projection and the Pi figures. **Allergy slices 1–3 MERGED** (the bullets above). **Next: charting a
>    medication in the window** (design APPROVED 2026-11-11, three PRs: the line + the check at charting; stat ordered →
>    given; aliases + PBS forms as drugref supplies them). **Slice 1a MERGED in PR #1066** (the bullet above);
>    **now: batch charting (#1072)** — design APPROVED 2026-11-11 ([`superpowers/specs/2026-11-11-batch-charting-1072-design.md`](superpowers/specs/2026-11-11-batch-charting-1072-design.md), three slices: the list + one Sign; versions + set-aside event + From:/why; the threshold); **slice 1 PLANNED** ([`superpowers/plans/2026-11-11-batch-charting-slice-1-the-list-and-one-sign.md`](superpowers/plans/2026-11-11-batch-charting-slice-1-the-list-and-one-sign.md), draft PR #1078), to build next. **Then 1b**: the same-drug de-label offer
>    on *Already taking* (#1053, opens db/075's `tolerated.coding`) and the group comment, a new allergy event type,
>    in one transaction with the line (design §3.5); it closes #1054. Then slice 2 (stat ordered → given, #1069). Later picks: slice 4
>    (the emergency key); **#1014** (first if a multi-node pilot nears); #1021's projection. Design §10's findings 1–2
>    are decided (ADR-0088), 3 is deferred (#979), 4–5 are **#965**, **#964**.
>    Design: [`superpowers/specs/2026-11-03-sign-in-slice-3-notes-drafts-and-whos-here-design.md`](superpowers/specs/2026-11-03-sign-in-slice-3-notes-drafts-and-whos-here-design.md)
>    (PR #930, merged; it also carries the per-note "unconfirmed" marker and #918's display half). The maintainer picks
>    the order of **#909** vs slice 5 (either gates a multi-terminal pilot) and of **#912**.
>    Owed by a human: switch the measurement cluster to `scram-sha-256` before the next RUNBOOK run (§0); **RUNBOOK
>    §16** (sign ≤ 3 s, next-nurse switch, VoiceOver/keyboard on the strip, the terminal lock), **§17**, **§18**, **§19**, **§20**, **§21**, **§22**, **§23**, **§24**, **§25**, **§26**, **§27**, **§28**, **§29** (allergies); #838's
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
> [`developers/durable-rules.md`](developers/durable-rules.md) carries every slice's "do not undo" block (charting 1a, allergy 3, allergy 2, allergy 1, sign-in 3c-3c, 3c-3a, 3c-3b, 3c-2, 3c-1, 3b-2b, 3b-2a, 3b-1, 3a, 2, 1, #898/#899, #745·#752·#740, R7, R6, R5b,
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

**Session date:** 2026-11-11 (#1072 batch charting brainstormed → design approved → slice 1 planned, draft PR [#1078](https://github.com/cairn-ehr/cairn-ehr/pull/1078)) · 2026-11-11 (charting slice 1a plan → SDD, Tasks 1–7 + final fix wave on draft PR [#1066](https://github.com/cairn-ehr/cairn-ehr/pull/1066); #1067–#1073 filed) · 2026-11-10 (#1052 merged; #1065 + drugref#231 filed; charting-a-medication brainstorm) · 2026-11-08 (#1021 measured; key facts once per key; keep rule) · 2026-11-07 (3c-3c plan → SDD, 7 tasks + final fix wave, PR [#1019](https://github.com/cairn-ehr/cairn-ehr/pull/1019)) · 2026-11-07 (3c-3a plan → SDD, Tasks 1–9 + final fix wave, PR [#1013](https://github.com/cairn-ehr/cairn-ehr/pull/1013)) · 2026-11-07 (3c-3 design; 3c-3b, PR [#1006](https://github.com/cairn-ehr/cairn-ehr/pull/1006)) · 2026-11-06 (3c-2, PR [#1000](https://github.com/cairn-ehr/cairn-ehr/pull/1000)) · 2026-11-05 (3c-1, PR [#992](https://github.com/cairn-ehr/cairn-ehr/pull/992)) · 2026-11-04/05 (3b-2b, PR [#984](https://github.com/cairn-ehr/cairn-ehr/pull/984); 3b-2a, PR [#981](https://github.com/cairn-ehr/cairn-ehr/pull/981); 3b-1, PR [#966](https://github.com/cairn-ehr/cairn-ehr/pull/966)) · 2026-11-03 (slice 3a, PR [#931](https://github.com/cairn-ehr/cairn-ehr/pull/931); slice 3 design, PR [#930](https://github.com/cairn-ehr/cairn-ehr/pull/930)) · 2026-11-01/02 (slice 2, PRs #917 + #920; #926, PR #928; #898 + #899, PR #911) · 2026-10-31 (sign-in slice 1, PR #897) · earlier (PRs #724–#895, 2026-10-04 → 10-31; R1–R3, the funnel, DR): ROADMAP. ·
**Spec:** **v0.101** (newest ADR-0099, PR #1066, `db/077`; v0.100 ADR-0098, PR #1052, `db/076`; v0.99 ADR-0097, PR #1038, `db/075`; v0.98 ADR-0096, PR #1026, `db/074`; v0.97 ADR-0095, PR #1019, `db/073`; v0.96 ADR-0094, PR #1013, `db/072`; v0.95 ADR-0093, PR #1006, no SQL change; v0.94 ADR-0092, PR #1000; v0.93 ADR-0091, PR #992; v0.92 ADR-0090, PR #984; v0.91 ADR-0089, PR #981; v0.90 ADR-0088; v0.89 ADR-0087; v0.88 ADR-0086; v0.87 ADR-0085; v0.86 ADR-0084; v0.85 ADR-0083; #857 and #861 needed no ADR — a projection-side fix, no wire change;
[ADR-0067](spec/decisions/0067-a-restore-reads-the-clinical-plane.md) supersedes ADR-0026 decision 2's implementation
wording only) · **`SCHEMA_GENERATION`:** **77** (`db/077`, PR #1066, with db/031 + db/037 + db/044 in place and its search door in db/076 §5; `db/076`, PR #1052, with db/074 + db/041 + db/042 in place; `db/075`, PR #1038; `db/074`, PR #1026; `db/073`, PR #1019; `db/072`, PR #1013, which also edits db/004 and db/052 in place; `db/071`, PR #1000, which also edits db/037 and db/044 in place; `db/070`, PR #992, which also edits db/064 and db/065 in place; `db/068` + `db/069`, PR #984, which also edits db/037, db/044, db/065 and db/066 in place; `db/067`, PR #981, which also edits db/065 and db/066 in place; `db/066`, PR #966; `db/064` + `db/065`, PR #931, which also edits db/044 and db/037 in place; `db/063`, PRs #917/#920; `db/062`, PR #911; `db/061` PR #897; `db/060`, PR #860; R6 and R7 edit db/054, R7 db/056, in place) · **Phase:** architecture
complete; **first production clinical surface RUNNING** — `cairn-node` plus a Tauri 2 window: the funnel front door (one
row per person) onto a medication chart that reads linked charts as one; a per-node matcher worker proposes duplicates,
a banner on the chart shows each one for a human to judge, a tray at the front door lists them all, and a
doubted link can be confirmed from the window; clinicians sign in per act and write, autosave and sign a progress note,
several writers to a draft, an untouched draft goes in unsigned by itself, a clinician can discard their own draft, leaving a signed record of it, and can add to a signed note, which then reads as one note with its parts; a part can be signed later, by its writer or by a supervisor's sign-off (PR #1000), and a part signed by a since-repudiated key counts as unsigned until its real writer re-signs (PR #1006); supervision is recorded, a supervisee's part needing a co-sign waits for a supervisor, and a supervisor can co-sign a supervisee's note as reviewed (PR #1013); the person named in who's here sees in one menu what waits for their signature or co-signature (PR #1019).

**Built so far** (orientation only; ROADMAP + ADR log + git carry the detail): demographics 1–5 · §5.2 advisory Python
matcher · §5.7 identity core C1–C5 (C5+ `reattribute` waited on a clinical-note surface, which 3a now gives it) · §5.4 John-Doe (§5.12
push-alert open) · §5.3/§5.8 funnel (ADR-0061; precedence #345 at db/005 step 8b; ranking ADR-0075; one row per
PERSON since R3, PR #721) · `clinical.medication` 1–6b under born-sealed bodies (ADR-0052) + per-write human authorship
(ADR-0053 — grading half-live until #245) · §5.9 stream through its read surface · the text progress note `clinical.note.asserted` + its draft store (ADR-0087, slice 3a), per-writer parts (ADR-0088) the quarantine commit + `Asserted` (ADR-0089) the audited discard (ADR-0090) and the compound note + addendum (ADR-0091) and the vouch + sign-off (ADR-0092, PR #1000) · med-list node tier (read + whole-list
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

### 2026-11-08 — #1021: the person menu measured at scale, 5–6× cut, then a keep rule

Rig `examples/measure_person_menu.rs` (seeds a ward through `sign_note` in stages; `--measure-only` re-times a node,
`--later-days 8` reads it as a ward's steady state); figures in
`crates/cairn-node/results/2026-11-08-person-menu-reads-macos-m3max.md`. Final state: the lib + 30 note/supervision DB
suites (756 tests, none skipped) and the GUI tree's DB-gated `--bins` (787) green. Finalcheck review then made the
keep rule's two hand-kept contracts structural: `MenuRead` carries `read_at` (`menu_for`/`counts` take no `now`, so
they cannot judge at an instant the read did not cut at), and a unit test pins its `'repudiated'` to
`vouch_read::CONFIRMATION_WORDS` as `signer_of` reads them; plus a test for the co-sign half of the drafted term.
- **⇒ Profile before choosing the big fix.** The issue's proposed fix was a new projection. `track_functions` showed
  88% of the cost was ~0.9 M calls of `SECURITY DEFINER` key doors (never inlined; 1.4 M with the per-part attester
  door) for **40** distinct keys: a CTE fixed that without touching meaning. Prove "identical" with `EXCEPT ALL` both ways on the big fixture before the rewrite.
- **⇒ A time limit on what is OWED hides it; a time limit on what is SHOWN is free.** The maintainer proposed
  cutoffs (1 month mandatory, 1 week optional). Group 3 already had the week (Rust-side, now pushed into SQL); a cutoff
  on groups 1–2 would drop an unsigned or un-co-signed note from "Waiting for you" at day 31, and a monthly-synced
  clinic's notes would arrive already expired. The lossless form is a SUPERSET keep rule, each term traced to a group.
- **⇒ Mutation-check a superset filter, and check the mutation tool too.** The 30-days-on re-read pins each term
  (removing any one fails a test). The first mutation run replaced terms with `FALSE`, orphaning `$2`/`$3`, so every
  test failed on a parameter-type error, not the filter: use `(FALSE AND term)`.
- **⇒ Fact-check your own figures** before they land: a nested function's *total* double-counts (94% → 88% self time),
  and a ratio projected from one statement under-shot the composed read (0.9 s projected, 1.05 s measured).

### 2026-11-07 — slice 3c-3c (PR [#1019](https://github.com/cairn-ehr/cairn-ehr/pull/1019)), condensed

Plan → SDD, 7 tasks, five fix rounds, an ADR fact-check (1 false + 8 imprecise sentences), whole-branch review.
- **⇒ A count belongs to a person: hide it the moment the claim changes**, before the new read.
- **⇒ A budget's fixture must be able to reach the cliff** (60 notes could not; #1021 above).
- **⇒ Grep the payload before adding a field**; **a plan's "one exception" to a house rule is a plan defect**;
  **"as on the chart" is a claim about the sort KEY, not only the tie-break.**

### 2026-11-06/07 — slices 3c-2 (PR #1000), 3c-3b (PR #1006), 3c-3a (PR #1013)

Condensed 2026-11-07; each plan → SDD, a final review, one fix wave; RUNBOOK §23–§25 owed (human).
- **⇒ Two clocks in one comparison is a finding, even when each is right** (#1014). **A fallback must accept what the
  floor can ADMIT on that part**, or it nags forever. **"Who wrote it" has one predicate per question**
  (`writer_actor` vs `writer_signature`). **The controller's own copy is a claim too.**
- **⇒ Walk an overlay BEFORE and AFTER its repair** (#1009). **A test filter is a claim about which tests ran**
  (`--bins notes_` matched nothing yet printed "passed"). **The floor's identity helpers answer different questions**
  (`cairn_note_key_actor` ignores repudiation; `cairn_key_confirmation` names it).
- **⇒ A new act can break an invariant the old acts only implied** (`signInFlight()`; the medication pane still is
  not, #1002). **A Review Focus test must set up that focus's scenario** (key and claim must differ). **A defensive
  applier must hold the floor's closed vocabularies.** **A plan's outcome sentence is a claim about every state.**

### 2026-11-04 → 11-05 — sign-in slices 3b-2b (PR [#984](https://github.com/cairn-ehr/cairn-ehr/pull/984)) and 3c-1 (PR [#992](https://github.com/cairn-ehr/cairn-ehr/pull/992))

Condensed 2026-11-06; each was plan → SDD, every task reviewed twice, its rulings in its ADR (0090, 0091) and its
durable-rules block. 3c-1's PR review (5 agents) added `DiscardOutcome::NeverSaved`, two mutation-checked DB tests and
ADR-0091 (g); it commented #994, #946, #995, #996.
- **⇒ A pin on a condition's TEXT does not pin WHEN it is evaluated** (a dead "Add to this note" after every reopen);
  pin the ORDER and walk it. **A door that delegates must not leave a half-made row** on the delegate's refusal.
  **Take a persisted fact from where it is persisted** (Sign took the target from the window; a restarted one sends
  none). **"Penned until the target lands" holds only where the floor can read the body** (#995).
- **⇒ A plan's COPY borrowed from a sibling gesture is a claim about THIS gesture**, false three times in 3b-2b (Sign's
  leftover is #986). **A signature proves "this key signed A discard", not "this one"**: bind the event to an instant
  the door mints under the lock (residual #987). **"This press may have taken the text" is not "everything after it
  went too"**: each press remembers what it carried (`pressedText`), and a press that consumes a draft with no spend
  row waits for the save queue (`notesSaved`).

### 2026-11-01 → 11-04 — sign-in slices 2 (PRs #917 + #920), 3a (PR [#931](https://github.com/cairn-ehr/cairn-ehr/pull/931)), 3b-1 (PR [#966](https://github.com/cairn-ehr/cairn-ehr/pull/966)), 3b-2a (PR [#981](https://github.com/cairn-ehr/cairn-ehr/pull/981))

Condensed 2026-11-05; each was plan → SDD, every task reviewed twice, its rulings in its ADR (0085, 0087, 0088, 0089)
and its durable-rules block. Slice 2's two real cases (a junior's password sent to an inbox that needs it; a locum's
Friday-night start) turned "re-key by token" into in-person enrolment.
- **⇒ A plan's own wording can be the defect** (blank is `cairn_twin_is_present`, never `btrim(…) <> ''`); "own part"
  by key, not by text. **An escape is a wire decision, and so is the twin of an escaped text.** Refusing a newer
  profile at the remote door wedges sync; admitting it unchecked is safe only if every reader reads the projection.
- **⇒ "Spent by a note" means "by THIS transaction's note"** (`xmin`; no savepoint). **"Is this key current"** asks
  *now* at admission, *does it still vouch* on re-judging. A fixture can make its assertions unreachable.
  **Fact-checking the ADR is a code review** (it found four window gaps the task reviews passed).
- **⇒ A door's helpers live in its own file, ahead of it**; `CREATE OR REPLACE` cannot change a return shape; run a
  door split across tasks as one dispatch. **Walk every path that LEAVES the editor** and every sentence through every
  branch (#970). A failure the user can act on must reach the user, not only stderr; a new `cairn-gui/*/src` file is
  read by the ROOT tree's source guards; a displayed time is computed from the door's own input; walk an offered remedy
  through the refusal it meets; "human" is the registry's actor KIND, not "nameable".
- **Mechanics:** a harness background job can be SIGTERMed (exit 143) — run the 2 h gate detached (`nohup … & disown`)
  in a worktree pinned to the PR head, with a done-marker; a second scratch DB set (`cairn_sdd/2/3` on :5532,
  `cairn_pgx` + the scratch marker) lets DB work proceed beside the gate without the `cairn_test` DDL deadlock.

### 2026-10-31 → 11-01 — sign-in slice 1 (PR [#897](https://github.com/cairn-ehr/cairn-ehr/pull/897)), #893 (PR [#895](https://github.com/cairn-ehr/cairn-ehr/pull/895)), #898 + #899 (PR [#911](https://github.com/cairn-ehr/cairn-ehr/pull/911))

Condensed 2026-11-05; each was brainstorm → design → plan → SDD, every task reviewed. Slice 1 filed #898–#902; #911
filed #909, #910, #912. Durable-rules carries each block.
- **⇒ A transactional counter is no throttle against a raw-SQL client** (`BEGIN; guess; ROLLBACK;`): count in state a
  rollback cannot undo (sequences, `setval`), BEFORE revealing anything; a clock stepped BACK must allow, not lock out.
- **⇒ Check WHICH role the client actually connects as** (the window was the owner, #898). `has_*_privilege` sees only
  INHERITED power: judge every role the login is a MEMBER of, and `session_user`. A definer calling helpers by bare
  name runs whoever can `CREATE` in `public` (db/062 revokes it).
- **⇒ An issue's suggested fix is a claim** (`SET ROLE` after the load closes nothing); `pg_hba_file_rules` is the FILE,
  not the loaded rules. **A permission walk on EMPTY data proves nothing**; removing a call removes its side effects
  (#912). An aborted tokio-postgres task gives every pending query `Error::closed()` (#893).

### 2026-10-04 → 10-30 — R4–R7, cloud passes, #797, the W, interrupted commits, the lost connection (PRs #724–#890)

Condensed 2026-11-07 (ROADMAP has each PR, durable-rules each slice's rules): R4–R7 (PRs #724–#788), the cloud passes
(#770–#776), #797 (PR #805; the maintainer's rule *tell the clinician only what changed from ANOTHER source since they
took the chart*), the W (#831 + #838), interrupted commits and the lost connection (PRs #813–#895). **Still open from
them:** #220 · #427 · #585 (nothing reads Postgres notices) · #712 · #718 · #738 · #755 · #756 · #757 · #778 · #784 ·
#785 · #790 · #791 · #803 · #804 · #812 (items 7–10) · #837 · #891; **decisions owed:** #613 · #628 · #649 · #780
(`casefold()`) · #825 · #850 · #866 · #867 · #870 · #871. Turkish dotless `ı` is the ADR-0014 profile's job.
- **⇒ A timeout around a tokio-postgres call does not free the connection**: abort the connection TASK and
  `pg_terminate_backend` the old backend; a timeout around a call with its own must be LONGER. A deferred trigger
  that sleeps makes COMMIT the slow statement.
- **⇒ Route a cause by TYPE end to end** (`RefusalScope::Stale` → `DataError::Stale` → `Retry::AfterReread`); a new
  meaning gets a new field, not a status-dependent reading of an old one; a `From` impl makes `?` compile where it is
  wrong. **An outcome that MAY have written settles like a success**; a floor `P0001` carries no scope — re-ask it,
  never parse it (#855). The logical clock is not a wall clock (#803). Two humans with the same determinants are ONE
  actor (#152).
- **⇒ Walk every remedy, absence or future-event sentence through every state it is shown in**; a remedy that names a
  command is a claim (run it, name the LIGHTEST, #787); count steps against the as-built window (#754); read the
  design's filter against its issue (#700); diagnose before choosing a fix (#725, `EXPLAIN ANALYZE`).
- **⇒ "Never over-erase" is half a rule**: a scrub of a winner-takes-all projection owes a re-fold. A new projection
  beside a one-winner one needs a second apply fn, a generation bump, the shred delete, BOTH registry counts.
- **⇒ Check a mutation APPLIED before trusting the green after it**; walk the OLD code with the same script; webview JS
  is pinned without a JS runner (`source_guard::js_fn`). A real mouse finds what `element.click()` cannot.
- **⇒ #726: a connect replays the schema only when it changed** (ADR-0080; tests call `db::force_schema_replay`).
  Also: a case-mapping change on one side of a subset invariant breaks it (#775); COMMITTED IS NOT VOUCHED (#556); a
  judgement carries what was compared (#708); an unknown outcome is its own ROOT marker (#713); a per-row lock cannot
  guard a whole-log scan (#773); an issue's fix can be wrong about MVCC (#560) or predate a guard (#608 vs #633); a
  count is not a set (#553); a lenient arm can be worse than silent (#768, #606). A title is a claim (ADR-0078); one
  in-memory truth (#786).
- **⇒ ROLLBACK HAZARD:** with `SCHEMA_GENERATION` unchanged, an OLDER binary on a database this branch loaded fails
  "cannot change return type" on db/056 — `DROP FUNCTION cairn_duplicate_check_status();` before switching back.
- **Cloud rig** (runs the FULL DB gate): micromamba + conda-forge `postgresql=18`, `cargo install cargo-pgrx --version
  =0.18.1 --locked`, `cargo pgrx init --pg18 <conda pg_config>` with an `xlocale.h` shim in `CFLAGS` and
  `BINDGEN_EXTRA_CLANG_ARGS`, `initdb` as `postgres`, `CREATE ROLE root SUPERUSER`; never pre-create `cairn_pgx` in
  `template1`; `apt-get install libwebkit2gtk-4.1-dev libgtk-3-dev libsoup-3.0-dev`; `target/` ~23 GB; a new db/tests
  mirror needs `\ir _scratch_database_guard.sql`. `cargo doc -D warnings` reads `<date>` as an unclosed HTML tag; a
  test assert must never echo identity-built text (CodeQL failed #795 after merge).

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
