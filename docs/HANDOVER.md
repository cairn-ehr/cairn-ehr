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
> **Its parts are ALL MERGED (2026-10-31 → 11-15).** Each ADR, ROADMAP and each `durable-rules.md` block carry the
> detail; this keeps what is still open, per slice. Never drop a number here while condensing.
> - **Sign-in / authorship** (design
>   [`superpowers/specs/2026-10-31-gui-sign-in-and-authorship-design.md`](superpowers/specs/2026-10-31-gui-sign-in-and-authorship-design.md),
>   builds ADR-0008 §5.11; slices 1 sign-in → 2 re-key (#247, #664 first) → 3 drafts + `Asserted` grade + its display →
>   4 emergency key → 5 autonomous terminal nodes that sync credentials with specified nodes). **1 → 3c-3c merged
>   2026-10-31 → 11-08**; RUNBOOK §16–§26 owed by a human. Warnings that outlive them: **ruling R4** — remote apply
>   admits any key in `actor_key_vouches`, so a leaked, rotated-away, UNrepudiated key could inject events via a peer or
>   medium; **a local-state export by this build will not decode on an older build**, and one carrying a supervision row
>   is one-way. Open issues by slice:
>   **1** (PR #897, [ADR-0083](spec/decisions/0083-the-human-key-vault.md), `db/061`): see the #909 box below.
>   **2** (PRs #917 + #920, [ADR-0085](spec/decisions/0085-in-person-enrolment-and-re-key.md), `db/063`; #926 in
>   [ADR-0086](spec/decisions/0086-who-may-act-after-a-repudiation.md); #919 + #924 accepted): #918 · #921 (a decision) ·
>   #922 · #923 · #925 · #927 · #929.
>   **3a** (PR #931, [ADR-0087](spec/decisions/0087-the-first-note-stream-and-its-draft.md), `db/064` + `db/065`; read
>   notes only from `clinical_note`, R8; `note.added` stays, #364): #945 (half built) · #946 · #947 · #948 (ruled
>   penned, ADR-0091) · #949 · #950 · #951 · #952 · #953 · #957 (a decision) · #959 · #960 · #961 · #962; RUNBOOK §18
>   half run 2026-11-03 (filed #954, #955).
>   **3b-1** (PR #966, [ADR-0088](spec/decisions/0088-whos-here-and-a-drafts-parts.md), `db/066`; #958, #973 built):
>   #963 (slice 5) · #964 · #965 · #967 · #968 · #969 · #970 · #971 · #972 · #975 · #976 · #977 · #978.
>   **3b-2a** (PR #981, [ADR-0089](spec/decisions/0089-the-quarantine-commit-and-the-asserted-grade.md), `db/067`;
>   #974 ruled not `M > N`, #982 built): #979 (deferred) · #980 · #983.
>   **3b-2b** (PR #984, [ADR-0090](spec/decisions/0090-the-audited-discard.md), `db/068` + `db/069`): #985 · #986 ·
>   #987 · #988 · #989 · #990 · #991.
>   **3c-1** (PR #992, [ADR-0091](spec/decisions/0091-the-compound-note-and-the-addendum.md), `db/070`): #993
>   (`K = 4 > N = 3`) · #994 · #995 · #996 · #998 · #999 (a decision).
>   **3c-2** (PR #1000, [ADR-0092](spec/decisions/0092-the-vouch-and-the-sign-off.md), `db/071`): #1001 · #1002 (the
>   medication pane reads the strip while a note act is in flight; PR #1142 closed it for sign-off, Stop, Withhold,
>   Release and the allergy presses — link and unlink remain) · #1003 · #1005.
>   **3c-3b** (PR #1006, [ADR-0093](spec/decisions/0093-a-repudiated-signature-counts-as-unsigned.md)): #1007 · #1008 ·
>   #1010 · #1011 (needs a gesture design) · #1012; #918's link half stays open. Design of 3c-3:
>   [`superpowers/specs/2026-11-06-sign-in-slice-3c-3-supervision-and-pending-signatures-design.md`](superpowers/specs/2026-11-06-sign-in-slice-3c-3-supervision-and-pending-signatures-design.md).
>   **3c-3a** (PR #1013, [ADR-0094](spec/decisions/0094-supervision-and-the-mandatory-co-sign.md), `db/072`): #1015 ·
>   #1016 · #1017 · #1018. Its #1014 (two clocks in the co-sign judgement) MERGED 2026-11-15 in PR #1124
>   ([ADR-0107](spec/decisions/0107-supervision-on-the-nodes-hlc.md), spec v0.109); the restore-door follow-on **#1125**
>   MERGED 2026-11-15 in PR [#1126](https://github.com/cairn-ehr/cairn-ehr/pull/1126); its final check filed #1127 (the six
>   supervision ops are hand-copied across db/004, db/072 and db/052). Not guaranteed: a CONCURRENT peer part from a clock ahead
>   can still sort after a lift it never saw (no local clock fixes that).
>   **3c-3c** (PR #1019, [ADR-0095](spec/decisions/0095-the-person-menu.md), `db/073`): #1020 (two actors sharing a
>   staff id) · #1021 (measured and cut 5–6× in PR #1025; open for the obligations projection and the Pi figures) ·
>   #1022 · #1023 (incl. `funnel.js`, 473 lines since PR #1142 moved `closeChart` out) · #1024; RUNBOOK §26 also owes WebKit + real Tauri IPC.
> - **Allergies** (design [`superpowers/specs/2026-11-08-allergy-stream-design.md`](superpowers/specs/2026-11-08-allergy-stream-design.md);
>   born-sealed, **no clear `safety` signal**, #406/#401).
>   **1** (PR [#1026](https://github.com/cairn-ehr/cairn-ehr/pull/1026), [ADR-0096](spec/decisions/0096-the-allergy-list.md),
>   `db/074`): #1028 · #1029 · #1030 · #1031 · #1032 (a decision) · #1033 (a decision) · #1034 · #1035 · #1036 ·
>   #1037; RUNBOOK §27.
>   **2** (PR [#1038](https://github.com/cairn-ehr/cairn-ehr/pull/1038), [ADR-0097](spec/decisions/0097-allergy-overlays-and-differing-accounts.md),
>   `db/075`; #1027 built): #1039 · #1040 · #1041 · #1042 · #1043 · #1044 · #1045 · #1046 (a decision) · #1047 · #1048 ·
>   #1049 · #1050 · #1051; RUNBOOK §28.
>   **3** (PR [#1052](https://github.com/cairn-ehr/cairn-ehr/pull/1052), [ADR-0098](spec/decisions/0098-allergy-coding-and-the-medication-check.md),
>   `db/076`, the one file that reads drugref): #1055 (drugref's cephalosporin rule — `no-match` is not "safe") ·
>   #1056 · #1058 · #1060 (a reconciled group with an uncoded member can read clean) · #1061 (a mild mark hides an
>   unverified severe allergen — ruling needed) · #1062 · #1063 · #1064; #1053, #1054, #1057, #1059 closed, #1065 →
>   charting 3; RUNBOOK §29 (copy `drugref_allergy` from :5532 — `drugref_profiles` has no groups).
> - **Charting a medication** (design [`superpowers/specs/2026-11-10-charting-a-medication-design.md`](superpowers/specs/2026-11-10-charting-a-medication-design.md);
>   "prescribe" means CHARTED, ADR-0099 decision 1; its three PRs are done).
>   **1a** (PR [#1066](https://github.com/cairn-ehr/cairn-ehr/pull/1066), [ADR-0099](spec/decisions/0099-charting-a-medication.md),
>   `db/077`; the override is general `warning_override {reason, against: [{source, ref}]}`): #1067 · #1068 · #1070 ·
>   #1071 · #1074 (**CI flake**: a schema-replay `40P01` between test binaries despite `test_serial_guard` — a red floor
>   job with that signature is it) · #1075 · #1076 · #1077; #1069 → slice 2, #1072 → batch charting, #1073 → slice 3;
>   RUNBOOK §30.
>   **1b** (PR [#1086](https://github.com/cairn-ehr/cairn-ehr/pull/1086), [ADR-0101](spec/decisions/0101-the-same-drug-delabel-offer.md),
>   the same-drug de-label offer): #1084 · #1085 · #1087; RUNBOOK §32. Maintainer 2026-11-13: the de-label row
>   pre-fills today's date only on an *Already taking* line ("tolerated today" is untrue on a *Start* line).
>   **2** (PR [#1106](https://github.com/cairn-ehr/cairn-ehr/pull/1106), [ADR-0105](spec/decisions/0105-a-stat-dose-ordered-then-given.md),
>   `db/082`, a stat dose ordered then given; rulings Q1–Q4 in the ADR): #1107 · #1108 · #1109 · #1110 · #1111 · #1112 ·
>   #1113 · #1114 · #1115 · #1116 · #1117 · #1118 · #1119 · #1120; RUNBOOK §36.
>   **3** (PR [#1121](https://github.com/cairn-ehr/cairn-ehr/pull/1121), [ADR-0106](spec/decisions/0106-drug-names-in-the-pick.md),
>   drug names in the pick; rulings R1–R3): #1122 · #1123; PBS forms (#1081) and brand → combination (drugref#244) wait
>   on data drugref does not have; RUNBOOK §37 (copy `drugref_synonyms`, not `drugref_allergy`).
> - **Batch charting, #1072** (design [`superpowers/specs/2026-11-11-batch-charting-1072-design.md`](superpowers/specs/2026-11-11-batch-charting-1072-design.md)).
>   **1** (PRs [#1078](https://github.com/cairn-ehr/cairn-ehr/pull/1078) + [#1083](https://github.com/cairn-ehr/cairn-ehr/pull/1083),
>   [ADR-0100](spec/decisions/0100-charting-a-list-under-one-signature.md), `db/078`): #1081 (a decision) · #1082 (a
>   decision); RUNBOOK §31.
>   **2a** (PR [#1088](https://github.com/cairn-ehr/cairn-ehr/pull/1088), [ADR-0102](spec/decisions/0102-set-aside-versions-and-the-source-of-a-line.md),
>   `db/079`, "From:" and set-aside versions): #1089 · #1090 · #1091 (a decision) · #1092 · #1093 · #1094 · #1095;
>   RUNBOOK §33.
>   **2b** (PRs [#1096](https://github.com/cairn-ehr/cairn-ehr/pull/1096) + [#1098](https://github.com/cairn-ehr/cairn-ehr/pull/1098),
>   [ADR-0103](spec/decisions/0103-settling-a-set-aside-version.md), `db/080`, restore and "checked: not current"):
>   #1097 · #1099 · #1100; RUNBOOK §34. Its rulings R3–R12 are in ADR-0103 and #1098's body — **the maintainer should
>   read them**.
>   **3** (PR [#1101](https://github.com/cairn-ehr/cairn-ehr/pull/1101), [ADR-0104](spec/decisions/0104-the-warning-threshold.md),
>   `db/081`, the warning threshold, `cairn-node medication-set-warning-threshold`; rulings Q1–Q3): #1102 · #1103 ·
>   #1104 · #1105; RUNBOOK §35.
>
> **#898 + #899 MERGED in PR [#911](https://github.com/cairn-ehr/cairn-ehr/pull/911)** (2026-11-01,
> [ADR-0084](spec/decisions/0084-the-window-never-holds-the-owners-keys.md), `db/062`): the window's own login
> (`cairn_window`), no schema load, no release over an over-powered login; `cairn-node provision-window-role` is the
> one owner step for install AND upgrade and refuses any `trust` rule. **⚠️ RUNBOOK §0:** the measurement cluster is
> `scram-sha-256`, and every window launch uses `$WINDOW_CONN`. Since ADR-0110 a remote database is "local, or
> pinned".
>
> > [!WARNING]
> > **NOT YET GUARANTEED — gates on any pilot:** until **#909** slice B (merged, PR #1163, ADR-0110) ships to a site, which waits
> > on slice C (the rollout), or slice 5 (every terminal its own node) lands, a facility has ONE usable terminal: the
> > one on the database's machine. **#912**: no
> > long-running process re-adjudicates deferred events (the window's launch used to, incidentally). **#910**: a
> > Cairn-installed Postgres (brainstorm first). From PR #911's second review: **#914** (say "cannot sign" at launch, not at the first signature) · **#915** (the posture check's routes are a hand-kept list) · **#916** (small robustness residuals); #913 was fixed in the PR. **Slice 5's design must answer** (design §4; and since 2026-11-04 how a draft reaches the facility's other terminal-nodes, partitioned case included — drafts never sync, **#963**): synced sealed blobs on a
> > stolen terminal disk can be guessed offline, and the attempt counter is per node. Open from slice 1: **#900**
> > (a superuser key could sign a clinical event) · **#901** (bound stored Argon2 parameters) · **#902** (handle case) ·
> > **#903** (another's guesses can hold a member at the 300 s cap — a decision) · **#904** (an unreadable blob counts
> > as a wrong guess; `cairn_pgx` 0.5.0) · **#905** (CI never runs `cairn-gui-tauri`'s DB-gated tests — run them
> > locally) · **#906** · **#907** · **#908** (a `cairn_pgx` reinstall re-grants `cairn_vault_unseal` to PUBLIC unseen).
> > Human-key `&SigningKey` parameters slice 5 must move onto the seam: `identify.rs:83`, `apply_proposal.rs:94`.
> > **Local `cairn_pgx` reinstall on this Mac:** patch the dylib with `scripts/macos-patch-pgx-dylib.py` (CI unaffected).
> > **Key-hold terminals:** a released key is held only under the claim of nobody or of its own person (#1128 — before
> > it, a key typed by someone else signed the named person's later presses). The prefilled ID still turns a
> > passer-by's passphrase into a counted try against the person named; since #1138 the refusal names that ID (the
> > back-off itself is #903).

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
> 1. **The whole-milestone review is DONE (2026-11-15)** —
>    [`code_reviews/2026-11-15-gui-milestone-review.md`](code_reviews/2026-11-15-gui-milestone-review.md), five seam
>    passes. **Fixed on PR [#1140](https://github.com/cairn-ehr/cairn-ehr/pull/1140):** #1128 (a held key signed for someone other than the person named — key-hold terminals),
>    #1129 (the vault and supervision doors were the advisory role's; [ADR-0108](spec/decisions/0108-the-human-doors-are-the-windows-role.md)),
>    #1131 (a hand tick held past a worse account of the same allergen). **Rulings owed by the maintainer:** **#1132**
>    (a student's timer-committed part escapes its co-sign — an ADR-0094 amendment), **#1141** (does an equal-severity
>    newer account unsettle a hand tick? today it does, the safe direction), **#1133** (an unsigned medication
>    list can be stranded), **#1134** (no log-out gesture: Lock keeps the claim and the editor), **#1135** (medication
>    co-sign for a supervisee; allergies' no-draft rule). **#1130 + #1137 MERGED 2026-11-15** in PR
>    [#1142](https://github.com/cairn-ehr/cairn-ehr/pull/1142) (`leave_chart.js`; durable-rules block; link/unlink still read the strip
>    outside the set — #1002, #709). Its finalcheck fixed an allergy press settling as "said nothing" (it now hands
>    back the words it said) and filed **#1143** (a press refused because another act is in flight says nothing — a
>    UI decision: disable the controls or say it beside them) and **#1144** (a refused close-time note flush loses the
>    text without a word; pre-existing). **#1139 + #1136 MERGED 2026-11-15** in PR
>    [#1145](https://github.com/cairn-ehr/cairn-ehr/pull/1145): `med_chart` deleted, its walks moved to `med_list_signed_db_tests.rs`;
>    `command_registry_tests.rs` now fails on a registered command the webview never calls (it reads `invoke("`,
>    `sendJudgement("`, `superuserAct("`, and fails on a call or row it cannot read); the CLI's `--help` says it checks no allergies (the medium fix — signing
>    `warning_override` from the CLI — is not built). **#1138 MERGED 2026-11-16** in PR
>    [#1148](https://github.com/cairn-ehr/cairn-ehr/pull/1148): items 1, 2, 4, 7, 8 built; 5 accepted (ADR-0098 d21);
>    9 commented on #1077. **Rulings owed: #1146** (item 3, a hold release past a new allergy) and **#1147** (item 6,
>    no audit trail on site policy). #1109 widened. **#909 slice A MERGED 2026-11-18 in PR #1152**
>    ([ADR-0109](spec/decisions/0109-the-terminal-service-and-the-keyless-window.md)): no window holds the node key;
>    `cairn-node terminal-service` registers from a queue (re-running the search against the BOUNDED prompt) and runs
>    the commit tick; filed **#1153**–**#1159** (#1159: the re-search guards only the queue). Final review and finalcheck
>    done and fixed. **Time-sensitive:** RUSTSEC-2026-0543 (chrono unmaintained) is ignored in both `deny.toml` trees until
>    2027-02-18 (**#1160**, migrate off chrono); **#389**'s glib ignore was re-decided 2026-11-20 to the same date (its exit
>    condition moved: gtk 0.19 + webkit2gtk 3 exist on glib 0.22; no tauri uses them yet).
>    **#909 slice B (the pinned gateway) MERGED 2026-11-23 in PR #1163** (plan PR #1162;
>    [ADR-0110](spec/decisions/0110-the-pinned-terminal-gateway.md), spec **v0.112**, `db/084`, generation **84**): a
>    remote window reaches the database only through `terminal-service --gateway-listen`, TLS 1.3 pinned both ways;
>    `gateway-setup`, `admit-terminal`, `revoke-terminal` (final; ends sessions, cancels statements), `list-terminals`;
>    the registration lock is the `registration_turn` row lock. Final review and finalcheck done and fixed (ADR-0110
>    and its durable-rules block carry them). Filed **#1164** (`connection.rs` is 650 lines: a verbatim split), **#1165**–**#1170** (no per-terminal
>    session cap · one host can hold all 128 pre-startup slots · no SCRAM test through the gateway · no cut-off-reopen
>    test · rustls close_notify log noise · a typed not-admitted marker), **#1171** (key-file mode unchecked off Unix;
>    `close_gently`'s unbounded shutdown), **#1172** (low-rated end-to-end test pins).
>    **#909 slice C1 (the rollout) MERGED 2026-11-24 in PR [#1176](https://github.com/cairn-ehr/cairn-ehr/pull/1176)**
>    (design/plan PR #1174, begun in PR #1173; [ADR-0111](spec/decisions/0111-an-older-window-works-above-a-floor.md),
>    spec **v0.113**, `db/085`, generation **85**): an older window works above a stamped floor and restarts into a
>    fitting install when idle or at Update now; the replay runs under a 2 s `lock_timeout`; the terminal service exits
>    75 on a new stamp. Filed **#1175** (db/043 swallows the re-adjudication's lock timeout) and **#1177** (the idle
>    re-lock closes a chart whose Sign settled badly — a decision).
>    **#909 slice C2 (a terminal follows its node) BUILT 2026-11-25 on draft PR
>    [#1181](https://github.com/cairn-ehr/cairn-ehr/pull/1181)** (on design/plan PR #1180; design
>    `superpowers/specs/2026-11-25-909-slice-c2-signed-release-design.md`, plan
>    `superpowers/plans/2026-11-25-909-slice-c2-signed-release.md`, 11 tasks by SDD;
>    [ADR-0112](spec/decisions/0112-a-terminal-follows-its-node.md), spec **v0.114**, `db/086`, `SCHEMA_GENERATION`
>    **86**): the steward signs window releases offline (`cairn-release`); `cairn-node release-intake` verifies and
>    stores them; the window fetches a fitting one through the gateway; a root-owned installer
>    (`cairn-terminal-install`) re-verifies it against the terminal's own pin and swaps it in. Filed **#1182** (the
>    genesis ceremony, a human act: no default pin until then), **#1183** (a node restored to an older release),
>    **#1184** (the installer's crash window after the swap), **#1185** (unknown artifact kinds never pruned), **#1186**
>    (the staging folder has no quota), **#1187** (co-signer floors on release load, ADR-0055 d6); the final review
>    filed **#1188** (after a release-signer rotation the node offers a kept release no terminal accepts), **#1189**
>    (terminal-wide refusals cost three downloads per session), **#1190** (three node outages give a release up until
>    restart), **#1191** (`a_press_waits_at_most_one_chunk` failed once under load; DB-gated, CI never runs it); the
>    finalcheck filed **#1192** (a withheld trust-root rotation keeps a rotated-out release key alive: a steward
>    decision, before the format freezes at #1182) and **#1193** (low-severity installer/intake robustness items).
>    **⇒ NEXT: the maintainer reviews #1180 + #1181 and merges; then the genesis ceremony (#1182).**
>    **Slice B must not ship to a site until slice C is settled.** RUNBOOK §38, §39 (two machines), §40 and §41 (a node
>    and a Pi) are owed by a human. **Otherwise:** sign-in
>    **slice 4** (the emergency key, design §5.6, brainstorm first); **#912**; **#1021**'s obligations projection and
>    Pi figures (ROADMAP has the Mac figures); or the next whole-project review. Design §10's findings 1–2 are decided
>    (ADR-0088), 3 is deferred (#979), 4–5 are **#965**, **#964**. Slice 3's design:
>    [`superpowers/specs/2026-11-03-sign-in-slice-3-notes-drafts-and-whos-here-design.md`](superpowers/specs/2026-11-03-sign-in-slice-3-notes-drafts-and-whos-here-design.md)
>    (PR #930; it also carries the per-note "unconfirmed" marker and #918's display half).
>    Owed by a human: switch the measurement cluster to `scram-sha-256` before the next RUNBOOK run (§0); **RUNBOOK
>    §16** (sign ≤ 3 s, next-nurse switch, VoiceOver/keyboard on the strip, the terminal lock), **§17**, **§18**, **§19**, **§20**, **§21**, **§22**, **§23**, **§24**, **§25**, **§26**, **§27**, **§28**, **§29** (allergies), **§30**, **§31** (charting), **§32** (the de-label offer), **§33** (From: and set-aside versions), **§34** (restore and checked), **§35** (the warning threshold), **§36** (a stat dose given and entered in error), **§37** (drug names in the pick), **§38** (the terminal service: the round trip it adds to Register ≤ 0.5 s median), **§39** (the pinned gateway, two machines: chart open and Sign ≤ 0.2 s slower remote, a revoke ≤ 2 s, `gateway-setup`'s success path), **§40** (the rollout: Update now ≤ 10 s, the service's restart ≤ 5 s on a remote window, no press past 15 s under an upgrade, the heal's longest lock), **§41** (a terminal follows its node, on a Pi: intake, download, a press ≤ ~200 ms behind a download, the swap, the refusals on screen, the rehearsed ceremony); #838's
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
> [`developers/durable-rules.md`](developers/durable-rules.md) carries every slice's "do not undo" block (#909 slice C2, #909 slice C1, #909 slice B, #909 slice A, #1138, charting 3, charting 2, batch charting 3, 2b, 2a, batch charting 1, charting 1b, charting 1a, allergy 3, allergy 2, allergy 1, sign-in 3c-3c, 3c-3a, 3c-3b, 3c-2, 3c-1, 3b-2b, 3b-2a, 3b-1, 3a, 2, 1, #898/#899, #745·#752·#740, R7, R6, R5b,
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
> required). **`fix(#500):` is SAFE** — the parenthesis breaks the adjacency. The `.githooks/commit-msg`
> hook runs the script BEFORE the commit exists (enable: `git config core.hooksPath .githooks`): the slip
> lives in the commit message and fixing it needs a force-push. Residuals **#547**, **#548**.

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
pass cleanly. Last full pass 2026-07-15 (#187–#217), fully closed. The runnable clinical surface had its first,
milestone-scoped review 2026-11-15 (#1128–#1139); the next whole-project pass should include it again.

> [!TIP]
> **The tech-debt loop is stopped, and stays stopped** (maintainer decision, 2026-08-09) while a human session
> holds the main repo — they contend on one cargo lock and one `test_serial_guard` advisory lock. **A live IDE
> contends the same way** (rust-analyzer holds `target/`); use a scratch `CARGO_TARGET_DIR=/tmp/…`, never kill
> the IDE. Loop gaps **#326**, **#312**, **#322**.

---

**Session date:** 2026-11-25 (#909 slice C2 designed, PR #1180, then built by SDD, Tasks 1–11, draft PR #1181; #1182–#1187 filed, then #1188–#1191 by the final review) · 2026-11-24, second session (#909 slice C1 built by SDD, Tasks 1–8, PR #1176 on design/plan PR #1174; #1175 and #1177 filed) · 2026-11-24 (slice C brainstormed → C1 design) · 2026-11-20 → 11-22 (#909 slice B planned, PR #1162, then built by SDD, Tasks 1–10, draft PR [#1163](https://github.com/cairn-ehr/cairn-ehr/pull/1163); #1164 filed) · 2026-11-17/18 (#909 slice A, PR #1152, ADR-0109) · 2026-11-16 (#1145 found merged; #1138's nine minors: five built, two commented, two split out as #1146 + #1147; PR [#1148](https://github.com/cairn-ehr/cairn-ehr/pull/1148)) · 2026-11-15, fifth session (#1130 + #1137 built, PR [#1142](https://github.com/cairn-ehr/cairn-ehr/pull/1142); its finalcheck fixed the allergy-words gap and filed #1143 + #1144) · 2026-11-15, fourth session (#1126 found merged; the whole-milestone review → #1128–#1139 filed;
#1128, #1129 (ADR-0108, spec v0.110) and #1131 fixed inline, PR [#1140](https://github.com/cairn-ehr/cairn-ehr/pull/1140)) · 2026-11-15, third session (#1124 found merged; HANDOVER condensed; #1125 fixed inline, PR [#1126](https://github.com/cairn-ehr/cairn-ehr/pull/1126) — db/052's restore door merges past an unstamped supervision row's fallback stamp and refuses a stamp on a non-supervision op; no ADR, no wire change, `SCHEMA_GENERATION` 82) · 2026-11-15, second session (#1121 found merged; #1014 planned → built inline, PR [#1124](https://github.com/cairn-ehr/cairn-ehr/pull/1124), ADR-0107) · 2026-11-15 (#1106 found merged; charting slice 3 (drug names) planned with rulings R1–R3 → built inline, PR [#1121](https://github.com/cairn-ehr/cairn-ehr/pull/1121); drugref#244 filed) · 2026-11-14, third session (#1101 found merged; charting slice 2 planned with rulings Q1–Q4 → SDD Tasks 1–7 + a whole-branch review on draft PR [#1106](https://github.com/cairn-ehr/cairn-ehr/pull/1106); #1107–#1112 filed) · 2026-11-14, second session (#1098 found merged; batch-charting slice 3 planned → built inline, PR [#1101](https://github.com/cairn-ehr/cairn-ehr/pull/1101); #1102 filed) · 2026-11-14 (#1088 merged; slice 2b planned → SDD Tasks 1–8, Tasks 1–3 merged mid-session in PR [#1096](https://github.com/cairn-ehr/cairn-ehr/pull/1096), Tasks 4–8 on draft PR [#1098](https://github.com/cairn-ehr/cairn-ehr/pull/1098); #1097 filed) · 2026-11-13 (#1086 merged; batch-charting slice 2's wire brainstormed → designed → slice 2a planned → built by SDD, PR [#1088](https://github.com/cairn-ehr/cairn-ehr/pull/1088); #1089 + #1090 filed; final check: #1091–#1095 filed) · 2026-11-12 (charting slice 1b brainstormed → designed → built, PR [#1086](https://github.com/cairn-ehr/cairn-ehr/pull/1086), merged the same day; #1084 + #1085 filed) · 2026-11-11 (batch charting slice 1's Task 7 after its merge: ADR-0100, spec v0.102, RUNBOOK §31, a whole-slice review; #1081 + #1082 filed; PR [#1083](https://github.com/cairn-ehr/cairn-ehr/pull/1083)) · 2026-11-11 (#1072 batch charting brainstormed → design approved → slice 1 planned, draft PR [#1078](https://github.com/cairn-ehr/cairn-ehr/pull/1078)) · 2026-11-11 (charting slice 1a plan → SDD, Tasks 1–7 + final fix wave on draft PR [#1066](https://github.com/cairn-ehr/cairn-ehr/pull/1066); #1067–#1073 filed) · 2026-11-10 (#1052 merged; #1065 + drugref#231 filed; charting-a-medication brainstorm) · 2026-11-08 (#1021 measured; key facts once per key; keep rule) · 2026-11-07 (3c-3c plan → SDD, 7 tasks + final fix wave, PR [#1019](https://github.com/cairn-ehr/cairn-ehr/pull/1019)) · 2026-11-07 (3c-3a plan → SDD, Tasks 1–9 + final fix wave, PR [#1013](https://github.com/cairn-ehr/cairn-ehr/pull/1013)) · 2026-11-07 (3c-3 design; 3c-3b, PR [#1006](https://github.com/cairn-ehr/cairn-ehr/pull/1006)) · 2026-11-06 (3c-2, PR [#1000](https://github.com/cairn-ehr/cairn-ehr/pull/1000)) · 2026-11-05 (3c-1, PR [#992](https://github.com/cairn-ehr/cairn-ehr/pull/992)) · 2026-11-04/05 (3b-2b, PR [#984](https://github.com/cairn-ehr/cairn-ehr/pull/984); 3b-2a, PR [#981](https://github.com/cairn-ehr/cairn-ehr/pull/981); 3b-1, PR [#966](https://github.com/cairn-ehr/cairn-ehr/pull/966)) · 2026-11-03 (slice 3a, PR [#931](https://github.com/cairn-ehr/cairn-ehr/pull/931); slice 3 design, PR [#930](https://github.com/cairn-ehr/cairn-ehr/pull/930)) · 2026-11-01/02 (slice 2, PRs #917 + #920; #926, PR #928; #898 + #899, PR #911) · 2026-10-31 (sign-in slice 1, PR #897) · earlier (PRs #724–#895, 2026-10-04 → 10-31; R1–R3, the funnel, DR): ROADMAP. ·
**Spec:** **v0.114** (newest ADR-0112, PR #1181, `db/086`; v0.113 ADR-0111, PR #1176, `db/085`; v0.112 ADR-0110, PR #1163, `db/084`; v0.111 ADR-0109, PR #1152, `db/083`; v0.110 ADR-0108, PR #1140, db/061 + db/062 + db/063 + db/072 in place; v0.109 ADR-0107, PR #1124, db/004 + db/052 + db/072 in place; #1125 needed no ADR — a db/052 fix in place, meeting ADR-0107 decision 4's stated purpose; v0.108 ADR-0106, PR #1121, `db/076` in place; v0.107 ADR-0105, PR #1106, `db/082`; v0.106 ADR-0104, PR #1101, `db/081`; v0.105 ADR-0103, PRs #1096/#1098, `db/080`; v0.104 ADR-0102, PR #1088, `db/079`; v0.103 ADR-0101, PR #1086, `db/075` in place, no new file; v0.102 ADR-0100, PR #1078, `db/078`; v0.101 ADR-0099, PR #1066, `db/077`; v0.100 ADR-0098, PR #1052, `db/076`; v0.99 ADR-0097, PR #1038, `db/075`; v0.98 ADR-0096, PR #1026, `db/074`; v0.97 ADR-0095, PR #1019, `db/073`; v0.96 ADR-0094, PR #1013, `db/072`; v0.95 ADR-0093, PR #1006, no SQL change; v0.94 ADR-0092, PR #1000; v0.93 ADR-0091, PR #992; v0.92 ADR-0090, PR #984; v0.91 ADR-0089, PR #981; v0.90 ADR-0088; v0.89 ADR-0087; v0.88 ADR-0086; v0.87 ADR-0085; v0.86 ADR-0084; v0.85 ADR-0083; #857 and #861 needed no ADR — a projection-side fix, no wire change;
[ADR-0067](spec/decisions/0067-a-restore-reads-the-clinical-plane.md) supersedes ADR-0026 decision 2's implementation
wording only) · **`SCHEMA_GENERATION`:** **86** (`db/086`, PR #1181; 85 `db/085`, PR #1176; 84 `db/084`, PR #1163; 83 `db/083`, PR #1152; 82 `db/082`, PR #1106, with db/005 + db/020 + db/031 + db/037 + db/039 + db/048 + db/060 + db/077 + db/079 + db/080 in place; 81 `db/081`, PR #1101; 80 `db/080`, PRs #1096/#1098, with db/031 + db/037 + db/048 + db/078 + db/079 in place; 79 `db/079`, PR #1088, with db/031 + db/037 + db/048 + db/077 + db/078 in place; 78 unchanged by PR #1086, whose `db/075` edit is in place; `db/078`, PR #1078; `db/077`, PR #1066, with db/031 + db/037 + db/044 in place and its search door in db/076 §5; `db/076`, PR #1052, with db/074 + db/041 + db/042 in place; `db/075`, PR #1038; `db/074`, PR #1026; `db/073`, PR #1019; `db/072`, PR #1013, which also edits db/004 and db/052 in place; `db/071`, PR #1000, which also edits db/037 and db/044 in place; `db/070`, PR #992, which also edits db/064 and db/065 in place; `db/068` + `db/069`, PR #984, which also edits db/037, db/044, db/065 and db/066 in place; `db/067`, PR #981, which also edits db/065 and db/066 in place; `db/066`, PR #966; `db/064` + `db/065`, PR #931, which also edits db/044 and db/037 in place; `db/063`, PRs #917/#920; `db/062`, PR #911; `db/061` PR #897; `db/060`, PR #860; R6 and R7 edit db/054, R7 db/056, in place) · **Phase:** architecture
complete; **first production clinical surface RUNNING** — `cairn-node` plus a Tauri 2 window: the funnel front door (one
row per person) onto a medication chart that reads linked charts as one; a per-node matcher worker proposes duplicates,
a banner on the chart shows each one for a human to judge, a tray at the front door lists them all, and a
doubted link can be confirmed from the window; clinicians sign in per act and write, autosave and sign a progress note,
several writers to a draft, an untouched draft goes in unsigned by itself, a clinician can discard their own draft, leaving a signed record of it, and can add to a signed note, which then reads as one note with its parts; a part can be signed later, by its writer or by a supervisor's sign-off (PR #1000), and a part signed by a since-repudiated key counts as unsigned until its real writer re-signs (PR #1006); supervision is recorded, a supervisee's part needing a co-sign waits for a supervisor, and a supervisor can co-sign a supervisee's note as reviewed (PR #1013); the person named in who's here sees in one menu what waits for their signature or co-signature (PR #1019); allergies are recorded, coded and checked, and medications are charted one line at a time or as a list under one Sign (PRs #1026–#1078).

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

### 2026-11-25 — #909 slice C2, a terminal follows its node (SDD, draft PR #1181)
- **⇒ A root process reading what an unprivileged one wrote reads it as hostile** (`O_NOFOLLOW`, regular files only,
  every read capped) **and acts on the bytes it verified in memory**, never on the file again: the start check runs
  `<program>.new`, written from those bytes, because the staged file's owner could change it after the check.
- **⇒ A "never again" keyed on an unverified field is a lock-out weapon.** `refused_release` first recorded any
  refusal's release number; a forged meta naming the genuine release would have barred it. Only verdicts reached
  after verification may stick (R23). **Count attempts, not failures**, or a deterministic refusal loops (R19).
- **⇒ Delete by the key a row was written under, never by a content hash.** Pruning by digest deleted another
  release's identical row (R15).

### 2026-11-24 — #909 slice C1, the rollout (SDD, PR #1176)
- **⇒ `pg_blocking_pids` is empty once a `lock_timeout` has fired**; naming a blocker afterwards takes a heuristic
  ("may be holding it"). **⇒ Read the platform crate before promising a path** (Tauri caches it; on macOS a path
  through a symlink, `/tmp` included, reads as none). **⇒ Act only on a fresh read**: a cached stamp let a restart
  proceed into an outage. **⇒ Sample "is anything in flight" BEFORE an await.** **⇒ A plan-time premise is a claim
  too** ("the upgrade drops no connection" was false for every remote window); check a door snapshot against a FRESH
  database.

### 2026-11-20 → 11-22 — #909 slice B, the pinned gateway (plan PR #1162, PR #1163), and 11-16, #1138
- **⇒ At a protocol door, allow-list what passes**: Postgres applies every startup key as a setting AFTER the login
  was vetted (a bare `role=…`; a repeated key, honoured last but vetted first). **⇒ A transient condition never wears
  a permanent refusal's code** (57P03, never 28000), and a socket dropped with unread data can turn a refusal into an
  outage. **⇒ Closing a socket does not stop a busy backend**, so a revoke also cancels. **⇒ A `trust` test cluster
  cannot reach a verb that refuses `trust`** (`gateway-setup`'s success path is RUNBOOK §39's). **⇒ A fixture that
  builds half a state can hide the defect it should catch** (`pinned_dir()`'s pin with no key).
- **#1138 (11-16): a finding's "fix size" is a claim** — check it against every symptom it names; and "the floor
  still refuses it" must hold for EVERY rule the fallback covers (the site's expiry was read by no floor function).
  Filed: #1149 (the item-7 guard sees only named doors), #1150 (the line editor's check lacks the note), #1151 (a
  wipe says no reason).

### 2026-11-15 (fourth) — the whole-milestone review (#1128–#1139)
- **⇒ Per-slice reviews cannot see seams.** Five passes, one question each traced through every layer (who signs ·
  which chart · allergy ↔ medication · drafts at log-out · the floor as a whole), found one Critical and nine
  Important defects that ~20 slice reviews had passed. Verify each agent claim against the code before filing.
- **⇒ When a rule tightens, the old tests that go red may be pinning the defect.** Nine allergy DB tests signed as
  Bea under a fixture that names Sue and relied on Bea's key staying held — exactly #1128. Name the signer first
  (`here_as`), never loosen the rule.

### 2026-11-11 → 11-15 — charting 2–3, batch charting 1–3, #1014/#1125 (PRs #1078 → #1126): lessons
- **Floors and projections.** A reader's fallback for old rows must be applied by every door protecting that reader
  (#1125). An undo that reverses a projection needs its same-transaction pairing pinned at the floor (`xmin =
  pg_current_xact_id()`), and every reversible projection a two-connection race test (#855's keyed advisory lock). A
  column inside an HLC-winner-gated upsert is arrival-order dependent: provenance goes in its own insert-or-nothing
  projection. A deferred trigger runs outside the strict door's `SET cairn.remote_apply`: mark the door with a row.
  "Refused only at the authoring door" is wrong for a key whose presence has consequences; a JSON `null` refused at
  the remote door wedges a peer.
- **Invariants.** "Never two current lines" took four review rounds (#1098): walk every invariant over chains and
  siblings, and put the closure in ONE function both floor and window read. A test giving every member of a set one
  value pins none of them (#1127).
- **Words and defaults.** Outcome words come from the state after the act, never the act. A server-stamped field rides
  back on the save's own answer. A working-state default expires with the work; a default honest for one record kind
  can be a fabrication for its sibling. "Follows X's pattern" can import X's limit (#1102).
- **Tests and tools.** A plain fixture hides a `skip_serializing_if` field: build key-set fixtures at their richest. A
  CLI test can pass before the verb exists (exit 2 either way). A datetime-local fires no `input` while partial: walk
  with real Backspace. rust-analyzer re-locks `target/` on every save: clone it copy-on-write (`cp -c -R`) into a
  scratch `CARGO_TARGET_DIR`, connect tests as `$USER`.
- **Process.** A merged PR is checked against its PLAN's task list, never its title (#1078 merged without Task 7). A
  push reporting a new branch means the PR merged mid-session (#1096). Fact-check an ADR's sentences before review.

### 2026-11-06 → 11-08 — slices 3c-2, 3c-3b, 3c-3a, 3c-3c (PRs #1000, #1006, #1013, #1019) and #1021, condensed 2026-11-22

Each plan → SDD with a final review (3c-3c's ADR fact-check found 1 false + 8 imprecise sentences); RUNBOOK §23–§26
owed (human). #1021's rig is `examples/measure_person_menu.rs`, its figures
`crates/cairn-node/results/2026-11-08-person-menu-reads-macos-m3max.md`; `MenuRead` carries `read_at`.
- **⇒ Profile before choosing the big fix** (#1021: 88% of the cost was ~0.9 M calls of definer key doors for 40 keys,
  fixed by a CTE; prove "identical" with `EXCEPT ALL` both ways). **A time limit on what is OWED hides it; one on what
  is SHOWN is free** (a SUPERSET keep rule, each term traced to a group). **Mutation-check a superset filter, and the
  mutation tool too** (`(FALSE AND term)`, not `FALSE`). **Fact-check your own figures** (a nested total
  double-counts).
- **⇒ Two clocks in one comparison is a finding, even when each is right** (#1014). A fallback must accept what the
  floor can ADMIT; "who wrote it" has one predicate per question (`writer_actor` vs `writer_signature`); walk an
  overlay BEFORE and AFTER its repair (#1009); a test filter is a claim about which tests ran; a new act can break an
  invariant the old acts only implied (`signInFlight()`; the medication pane still is not, #1002); a Review Focus test
  must set up that focus's scenario; a plan's outcome sentence is a claim about every state.
- **⇒ A count belongs to a person: hide it the moment the claim changes**; a budget's fixture must reach the cliff;
  grep the payload before adding a field; a plan's "one exception" to a house rule is a plan defect.

### 2026-10-31 → 11-05 — sign-in slices 1 → 3c-1, condensed 2026-11-22

PRs #897 (1), #895 (#893), #911 (#898 + #899), #917 + #920 (2), #931 (3a), #966 (3b-1), #981 (3b-2a), #984 (3b-2b),
#992 (3c-1); rulings in ADR-0083–0091 and their durable-rules blocks. Slice 2's two real cases (a junior's password
sent to an inbox; a locum's Friday-night start) turned "re-key by token" into in-person enrolment. 3c-1's review
commented #994, #946, #995, #996; slice 1 filed #898–#902; #911 filed #909, #910, #912.
- **⇒ Pin WHEN a condition is evaluated, not only its TEXT** (#970); a plan's copy borrowed from a sibling gesture is
  a claim about THIS gesture (#986); bind a signature to an instant the door mints under the lock (#987); a door that
  delegates must not leave a half-made row. "Spent by a note" means by THIS transaction's note (`xmin`); blank is
  `cairn_twin_is_present`, never `btrim(…) <> ''`; an escape is a wire decision; fact-checking an ADR is a code
  review.
- **⇒ A transactional counter is no throttle against a raw-SQL client** (`BEGIN; guess; ROLLBACK;`): count where a
  rollback cannot undo, BEFORE revealing anything. Check every role the login is a MEMBER of (#898); a definer calling
  helpers by bare name runs whoever can `CREATE` in `public`; a permission walk on EMPTY data proves nothing; an
  aborted tokio-postgres task gives every pending query `Error::closed()` (#893).
- **Mechanics:** a harness background job can be SIGTERMed (exit 143): run the 2 h gate detached in a worktree pinned
  to the PR head; a second scratch DB set (`cairn_sdd/2/3` on :5532) lets DB work run beside it.

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
- **⇒ Route a cause by TYPE end to end** (`RefusalScope::Stale` → `DataError::Stale` → `Retry::AfterReread`); a `From`
  impl makes `?` compile where it is wrong; **an outcome that MAY have written settles like a success**; a floor
  `P0001` carries no scope — re-ask it, never parse it (#855); the logical clock is not a wall clock (#803); two
  humans with the same determinants are ONE actor (#152).
- **⇒ Walk every remedy, absence or future-event sentence through every state it is shown in**; a remedy naming a
  command is a claim (run it, name the LIGHTEST, #787); count steps against the as-built window (#754); read the
  design's filter against its issue (#700); diagnose before choosing a fix (#725). **"Never over-erase" is half a
  rule** (a winner-takes-all scrub owes a re-fold; a new projection beside a one-winner one needs a second apply fn, a
  generation bump, the shred delete, BOTH registry counts). **Check a mutation APPLIED before trusting the green**;
  webview JS is pinned by `source_guard::js_fn`; a real mouse finds what `element.click()` cannot.
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
  ([#935](https://github.com/cairn-ehr/cairn-ehr/issues/935)): the HLC drift ceiling trusts the local clock, so a node
  running behind refuses honest node-plane events (revocations included), mints corrections below their targets,
  aborts a DR restore onto a box whose clock is behind (db/009), and, more than 24 h behind its own HLC, refuses its
  *own* clinical writes (db/005; PR #932's review, #935's consequence 4 and option D, a local-door rule, since an
  offline node has no peers to ask). **Decision owed before S4 step 2 (plan D7, options A/B/C; B recommended):** where
  the per-node clock skew lives (a redefined `cairn_wall_ms()` does not survive `cairn-sync`'s replay, and the #726
  replay-skip does not fit one `node_schema` stamp shared by two loaders). ADR threads #937–#939; engineering #936,
  #940–#943.
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
