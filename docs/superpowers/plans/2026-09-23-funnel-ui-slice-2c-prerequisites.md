# Funnel UI slice 2c prerequisites — four traps closed before the window is written

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development
> (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use
> checkbox (`- [ ]`) syntax for tracking.

**Goal:** Close [#659](https://github.com/cairn-ehr/cairn-ehr/issues/659),
[#660](https://github.com/cairn-ehr/cairn-ehr/issues/660),
[#651](https://github.com/cairn-ehr/cairn-ehr/issues/651) and
[#654](https://github.com/cairn-ehr/cairn-ehr/issues/654) — four defects that exist **today** and
whose only consumer is the slice 2c window that has not been written yet — so that 2c is written
against firm ground rather than retro-fitted onto it.

**Architecture:** Two of the four are pure additions inside the `cairn-gui` tree (a settling
combinator on `TokenStore`; an injectable one-shot failure on the mock ports). Two are decisions in
`crates/cairn-node` that the GUI tree has nothing to read without: a **typed** deliberate-refusal
error so a Rust-side pre-flight refusal stops reading as an outage, and **one enrolment rule** —
no surface provisions an actor as a write-path side effect, `init` provisions, and a named command
is the remedy.

**Tech Stack:** Rust 2021, `anyhow`, `tokio`, `tokio-postgres`, PostgreSQL ≥ 18. No new dependency
in any tree. No migration, no `SCHEMA_GENERATION` bump, no ADR, no spec version bump.

**Spec:** `docs/superpowers/specs/2026-09-20-registration-search-funnel-ui-design.md` — in
particular its *Slicing* section (2b/2c split) and *Error handling* section's 2026-09-22 revision,
which is where three of these four were first written down as owed.

---

## Global Constraints

Copied verbatim from `CLAUDE.md` and from the durable rules slice 2b established. Every task's
requirements implicitly include this section.

- **AGPL-3.0**, and every dependency must be AGPL-3.0-compatible. **No new dependency is added by
  this plan** — if a task seems to need one, stop and ask.
- **TDD throughout.** The failing test is written and *run* before the code that makes it pass.
- **Inline documentation for a junior developer**: every non-trivial function carries *why it
  exists and how it fits*, not a restatement of the next line.
- **Never hard-code cryptographic material in tests, and never give a non-cryptographic value a
  cryptographic name.** Test keys are derived at runtime — `std::array::from_fn(|i| …)`. `salt`,
  `nonce` and `iv` are reserved for real constructions; a discriminator is a `lineage`, a
  `variant`, a `seed`. Enforced by `crates/cairn-node/tests/crypto_sink_names_are_genuine.rs`.
- **Three cargo trees, three lockfiles.** This plan adds no crate, so no lockfile moves. If one
  does move, `cairn-gui/Cargo.lock` and `extensions/cairn_pgx/Cargo.lock` must be refreshed too —
  only CI's `--locked` clippy on the GUI tree sees the staleness.
- **A DB-free `cargo test` requires `export CAIRN_ALLOW_DB_SKIP=1`** in **both** trees since
  2026-09-22.
- **`cairn-gui` is a separate cargo tree.** Its gate is roughly two minutes; the root tree's full
  local gate is roughly two hours over ~132 binaries. Run the narrow tests locally, let CI gate the
  root workspace. **Never `cargo test | tail`** — the pipe masks cargo's exit code.
- **A cross-crate signature change must be built with `cargo test --workspace`**, not
  `-p cairn-node`; `cairn-sync/tests/clinical_pull.rs` calls the node's orchestrators.
- **Check closing keywords before committing**, not after: run
  `scripts/check_closing_keywords.py`. A commit message saying *"Filed rather than fixed: #NNN"*
  auto-closes that issue, and fixing it needs a force-push, which is deny-listed.
- **`gh api` is deny-listed repo-wide.** Read CodeQL alerts with `scripts/codeql-alerts.sh`.
- **Never `git checkout -- <file>` to undo an edit.** It discards all uncommitted work in that
  file, silently and unrecoverably.

---

## File Structure

| File | Responsibility | Task |
|---|---|---|
| `cairn-gui/cairn-gui-funnel/src/token.rs` (modify) | Gains `TokenStore::settle` — the one combinator that makes the ergonomic path through a port's result the *correct* one | 1 |
| `cairn-gui/cairn-gui-data/src/mock/mod.rs` (modify) | `MockData` gains the one-shot failure slot and its setter | 2 |
| `cairn-gui/cairn-gui-data/src/mock/funnel.rs` (modify) | Both port impls consume the slot before answering from fixtures | 2 |
| `crates/cairn-node/src/db_diagnosis.rs` (modify) | Gains `DeliberateRefusal` + `deliberate_refusal()` + `is_deliberate_refusal()` — the typed answer to *"was this a verdict?"* for refusals raised in Rust | 3 |
| `crates/cairn-node/src/patient/register.rs` (modify) | `dob_precision` refuses with the typed error instead of a bare `anyhow!` | 3 |
| `cairn-gui/cairn-gui-live/src/error.rs` (modify) | `data_error_from` consults both discriminators — the SQLSTATE *and* the typed marker | 3 |
| `cairn-gui/cairn-gui-live/tests/refusal_is_not_an_outage.rs` (modify) | The pinned-wrong-behaviour test flips to expect `Refused` | 3 |
| `crates/cairn-node/src/actor_enrolment.rs` (**create**) | The one enrolment rule, in the library where both surfaces can reach it: probe, enrol, and the refusal that names the remedy | 4 |
| `crates/cairn-node/src/lib.rs` (modify) | Declares the new module | 4 |
| `crates/cairn-node/src/main.rs` (modify) | `enroll-device-actor` subcommand; `init` provisions; fifteen write commands *require* rather than provision; `ensure_registration_actor` deleted | 4 |
| `crates/cairn-node/tests/device_actor_enrolment.rs` (**create**) | DB-gated: idempotence, the kind-agnostic dual-mapping guard, and the refusal being a typed verdict | 4 |

---

## Paper-parity benchmark (§1.2)

Required by `CLAUDE.md` coding rule 7: task 4 changes a clinical workflow (registration at the CLI
gains a provisioning precondition). Tasks 1–3 are below the clinical surface and change no human
act — their rationale is in each task.

**Paper counterpart:** issuing a new clerk the key to the records room, and entering them in the
desk register, before they may file a card. A one-time act of the practice, not of a registration.

**Steps:**

| | Register a patient on a node nobody has provisioned |
|---|---|
| Paper acts (N) | 1 — the practice sets the desk up once when it opens |
| Architecture-forced (M) | 0 for the ordinary operator: `cairn-node init` enrols the device actor, so a node that was initialised is ready. **1** on a node restored without its actor registry, where it is a named remedy rather than a silent failure |
| UI bundling target (K) | 0 / 1 respectively |

`M ≤ N` in both columns, so there is **no architecture defect to file**. The count of human acts in
a *registration* is unchanged: this removes a hidden side effect, it does not add a step to the
workflow §1.2 measures.

**Time + cognitive load.** Zero change to the per-registration path — the probe is one indexed
`EXISTS` on `actor_current.signing_key_id` that replaces an `EXISTS` the CLI already ran on every
write command. Cognitive load *falls* on the failing path: today a fresh node's first GUI
registration reads `submit_event: signer 9f3c… is not an enrolled, non-revoked actor`, which names
a key and no remedy; after this it names the command to run. The end-to-end measurement this design
owes is unchanged and still belongs to slice 2c, which is the slice that first exposes a runnable
surface.

---

## Why these four, and why before 2c

Each is a trap whose only victim is code not yet written, which is exactly when it is cheapest to
close. Stated once here so the tasks do not each re-argue it:

- **#659** — the natural way to write 2c's command handler, `.map_err(|(e, _)| e)?`, compiles with
  no warning, drops the attestation and **latches `TokenStore` shut forever**. `discard()` does not
  clear `in_flight`, so the clerk editing the form does not recover; nothing short of rebuilding
  the store does.
- **#660** — `--mock` can never return `Err`, so 2c's two-armed refusal/outage rendering — the
  whole point of #648 — would ship with no test path at all in the mode the accessibility and
  timing passes run in.
- **#651** — a malformed date of birth (`3/2/1980` on a desk with no date widget) refuses in Rust
  with no SQLSTATE, so it reaches the clerk as an **outage with a retry button that can never
  work**. This is the default failure mode of the surface 2c builds.
- **#654** — the window's *first* registration refuses on a node where the CLI never registered
  anyone, and the message names a key rather than a remedy.

---

## ⚠️ A finding this plan makes that #654 does not

**#654 understates its own blast radius by fifteen.** Its text says *"`cairn-node
patient-register` calls `ensure_registration_actor`"*. In fact `ensure_registration_actor` is
called at **fifteen** sites in `main.rs`, and only one of them is `patient-register`:

`register-john-doe` · `patient-register` · `sensitivity-assert` · `assert-observed-evidence` ·
`assert-identity-evidence` · `identify-patient` · `medication-assert` · `medication-cease` ·
`medication-change-dose` · `medication-correct-dose` · `medication-code` ·
`medication-code-correct` · `medication-reconcile` · `medication-separate` · `shred`

Its own doc comment calls it *"the headless-node/CLI convenience"* — it is the CLI's **general
device-actor bootstrap for every authoring command**, not a registration thing. So the maintainer's
decision (*"provisioning command + retire CLI auto-enrolment, so there is one rule"*) necessarily
touches all fifteen: retiring it from `patient-register` alone would leave fourteen surfaces still
silently provisioning, which defeats the decision.

Two consequences this plan takes on deliberately:

1. **The command is named `enroll-device-actor`, not `enroll-registration-desk`.** The role string
   written into the enrolment stays `registration-desk` — changing it would change bytes in a
   signed actor event for no reason — but the *command* is named for what it does.
2. **`init` calls it.** The maintainer's option 1 offered *"or fold into `init`"*; this plan does
   **both**, because they answer different questions. `init` means a fresh node costs the operator
   no new act (paper-parity above: `M = 0`). The standalone command means a node restored without
   its actor registry, which never runs `init`, has a named remedy rather than a dead end.

**What this plan does NOT do, stated so it is not mistaken for done:** `LiveData` is left
unchanged. Its refusal is already correctly classified as `Refused` since #648, and making it
*actionable* means putting a sentence in the window's chrome — which is rendering, and rendering is
slice 2c's job. Task 4 exposes `device_actor_enrolled` publicly **so that 2c's `build_live_state`
can probe at launch**, which is #654's option 2 and the discipline `build_live_state` already
follows for the node key. That probe is 2c's, and it is named in the handover as owed.

---

## Task 1: `TokenStore::settle` — the short path becomes the correct one (#659)

**Files:**
- Modify: `cairn-gui/cairn-gui-funnel/src/token.rs` (add `settle` beside `restore`/`commit`, which
  are at `:372` and `:387`; tests go in the existing `#[cfg(test)] mod tests` at the foot of the
  file)

**Interfaces:**
- Consumes: the existing `TokenStore::{take, restore, commit, discard}`, `AttestedSearch`,
  `Restored::{Kept, SupersededAndDropped}`, `TokenError::RegistrationInFlight` — all already
  public.
- Produces:
  ```rust
  pub fn settle<T, E>(
      &mut self,
      outcome: Result<T, (E, AttestedSearch)>,
  ) -> Result<T, (E, Restored)>
  ```
  Task 2's end-to-end mock walk uses it, and slice 2c's command handler is expected to be its only
  production caller.

**⚠️ Why it is generic and not `Result<Uuid, (DataError, AttestedSearch)>` as #659 suggests.**
`DataError` lives in `cairn-gui-data`, and `cairn-gui-data` already depends on `cairn-gui-funnel`
(`port.rs` imports `AttestedSearch`). Naming `DataError` here would invert that edge into a cycle
and the tree would not build. Generic is not a compromise: `TokenStore` has no business knowing
what a failure *is*, only that one happened.

**⚠️ Do NOT also "fix" `discard`/`invalidate` to clear `in_flight`.** That reading of #659 is
wrong and it would reintroduce the bug `in_flight` exists to prevent. A clerk editing the form
while a registration is genuinely in flight must still have their second click refused — clearing
the flag there is how two clicks produced two charts. The correct fix is that *every* `take` is
guaranteed to reach `restore` or `commit`, which is exactly what `settle` guarantees.

- [ ] **Step 1: Write the three failing tests**

Append to the existing `mod tests` in `cairn-gui/cairn-gui-funnel/src/token.rs`. Reuse whatever
helper that module already has for building a `PromptList`; if it has none, build one with
`bound_for_prompt` exactly as the neighbouring tests do.

```rust
    // --- #659: settle is the only end of a `take` a caller can reach by accident ---

    /// A success settles the store, so the NEXT registration is not refused.
    ///
    /// Without `settle` the ergonomic `map_err(|(e, _)| e)?` leaves `in_flight` set and every
    /// later `take` returns `RegistrationInFlight` — for the rest of the window's life.
    #[test]
    fn settling_a_success_leaves_the_store_ready_for_the_next_registration() {
        let mut store = TokenStore::new();
        let first = record_a_search(&mut store, "Aabria Iyengar");
        let taken = store.take(first).expect("the only token");

        let settled: Result<u8, (&str, Restored)> = store.settle(Ok::<u8, (&str, AttestedSearch)>(7));
        assert_eq!(settled.ok(), Some(7));
        drop(taken); // the port consumed its copy; this one is the test's own

        let second = record_a_search(&mut store, "Bilal Osei");
        assert!(
            store.take(second).is_ok(),
            "a settled success must not latch the store: the next patient's registration is \
             refused forever otherwise"
        );
    }

    /// A failure settles it too, AND puts the attested search back for the retry.
    ///
    /// This is the design's *"Register fails. The form keeps its values."* — the clerk must not
    /// be made to re-search because the database hiccuped.
    #[test]
    fn settling_a_failure_restores_the_search_and_keeps_its_token() {
        let mut store = TokenStore::new();
        let token = record_a_search(&mut store, "Chidi Anagonye");
        let attested = store.take(token).expect("the only token");

        let settled: Result<u8, (&str, Restored)> = store.settle(Err(("the node was unreachable", attested)));
        let Err((message, restored)) = settled else {
            panic!("a failed registration must settle as a failure");
        };
        assert_eq!(message, "the node was unreachable");
        assert_eq!(restored, Restored::Kept);
        assert!(
            store.take(token).is_ok(),
            "the SAME token must still be redeemable, or the form's held handle is a lie"
        );
    }

    /// THE BUG #659 IS ACTUALLY ABOUT: the clerk edits while the registration is in flight.
    ///
    /// `discard` bumps the generation and leaves `in_flight` set, so the restore is correctly
    /// refused as superseded — but the store must still be SETTLED, or editing the form (the
    /// clerk's own recovery gesture) latches it shut and nothing short of rebuilding the
    /// `TokenStore` recovers.
    #[test]
    fn settling_a_failure_the_clerk_has_already_edited_past_still_unlatches_the_store() {
        let mut store = TokenStore::new();
        let token = record_a_search(&mut store, "Dara Ó Briain");
        let attested = store.take(token).expect("the only token");

        store.discard(); // the clerk corrects a typo while the write is in flight

        let settled: Result<u8, (&str, Restored)> = store.settle(Err(("refused", attested)));
        let Err((_, restored)) = settled else {
            panic!("a failed registration must settle as a failure");
        };
        assert_eq!(
            restored,
            Restored::SupersededAndDropped,
            "the pre-edit search must NOT come back — that is how a chart is born attesting a \
             search for a different spelling of the name"
        );

        let fresh = record_a_search(&mut store, "Dara O Briain");
        assert!(
            store.take(fresh).is_ok(),
            "the store must be usable again: editing the form is the clerk's recovery gesture, \
             and if it latches the store the recovery is the thing that breaks registration"
        );
    }
```

If `record_a_search` does not already exist in that test module, add it above the new tests:

```rust
    /// Record a search for one typed name and hand back its token.
    ///
    /// Every test below needs a redeemable token and none of them cares what was displayed, so
    /// the candidate list is empty — which is also the honest fixture for *"nothing fits, so
    /// register"*, the walk this store exists to serve.
    fn record_a_search(store: &mut TokenStore, name: &str) -> SearchToken {
        let query = SearchQuery::new(name, Some("1980-02-03"), &[]);
        let displayed = bound_for_prompt(&CandidateList::default());
        store.record(query, displayed).expect("a non-empty query")
    }
```

Adjust the `CandidateList` construction to whatever the neighbouring tests already use — read them
first rather than assuming `Default` is implemented.

- [ ] **Step 2: Run the tests to verify they fail**

```bash
cd /Users/hherb/src/cairn-ehr/cairn-gui && CAIRN_ALLOW_DB_SKIP=1 cargo test -p cairn-gui-funnel settle
```

Expected: FAIL to **compile**, with `no method named 'settle' found for struct 'TokenStore'`. A
compile failure is the correct red here — there is nothing to call yet.

- [ ] **Step 3: Write the implementation**

Add to `impl TokenStore`, immediately after `commit` and before `discard`, so the three ends of a
`take` read together:

```rust
    /// Settle a registration's outcome: **the only end of a [`TokenStore::take`] a caller can
    /// reach by accident, and it is the correct one.**
    ///
    /// # Why this exists rather than leaving callers to call `commit`/`restore` themselves
    ///
    /// [`PatientRegistration::register`] hands the attested search back *inside* its error,
    /// because [`TokenStore::restore`] needs exactly that value and nothing else in the program
    /// can obtain one. That makes the natural Rust idiom a trap:
    ///
    /// ```ignore
    /// let id = live.register(attested, name).await.map_err(|(e, _)| e)?;  // ← latches the store
    /// ```
    ///
    /// It compiles with no warning, discards the attestation during destructuring (so
    /// `#[must_use]` cannot help — it fires on unused *expression results*, not on dropped
    /// fields), and leaves `in_flight` set. Every later `take` then returns
    /// [`TokenError::RegistrationInFlight`], and because [`TokenStore::discard`] deliberately
    /// does not clear that flag, the clerk editing the form — their own recovery gesture — does
    /// not recover either. Nothing short of rebuilding the store does.
    ///
    /// Routing the outcome through here makes the short path the right one: the ergonomic call
    /// is now `store.settle(port_result)`, and `map_err(|(e, _)| e)` becomes the longer thing to
    /// write.
    ///
    /// # What it returns, and why the `Restored` is not swallowed
    ///
    /// On failure the caller gets the error **and** what became of the search:
    /// [`Restored::Kept`] means the form's token is still redeemable and the clerk may simply
    /// press Register again; [`Restored::SupersededAndDropped`] means a newer search landed or
    /// the clerk edited, so there is nothing to retry with and the window must wait for the next
    /// search before offering Register at all. Collapsing those two would put a live Register
    /// button over a search that no longer exists.
    ///
    /// # Generic on purpose
    ///
    /// `T` and `E` rather than `Uuid` and `DataError`: `DataError` lives in `cairn-gui-data`,
    /// which already depends on this crate, so naming it here would be a dependency cycle. It is
    /// also the honest signature — a token store has no business knowing what a failure *is*,
    /// only that one happened.
    pub fn settle<T, E>(
        &mut self,
        outcome: Result<T, (E, AttestedSearch)>,
    ) -> Result<T, (E, Restored)> {
        match outcome {
            Ok(value) => {
                self.commit();
                Ok(value)
            }
            Err((error, attested)) => Err((error, self.restore(attested))),
        }
    }
```

If `Restored` does not already derive `PartialEq`/`Debug`, add them — the tests compare it. Derive
rather than hand-writing: it is a fieldless enum.

- [ ] **Step 4: Run the tests to verify they pass**

```bash
cd /Users/hherb/src/cairn-ehr/cairn-gui && CAIRN_ALLOW_DB_SKIP=1 cargo test -p cairn-gui-funnel
```

Expected: PASS, with the three new tests named in the output and no existing test newly failing.

- [ ] **Step 5: Prove each new test by a mutation**

This crate's convention (slice 2a, 21 mutations) is that a test earns its place by failing against
a named mutation. Apply each, confirm the named test goes red, then revert it:

1. In `settle`'s `Ok` arm, delete `self.commit();` →
   `settling_a_success_leaves_the_store_ready_for_the_next_registration` must fail.
2. In `settle`'s `Err` arm, replace `self.restore(attested)` with
   `{ drop(attested); Restored::SupersededAndDropped }` →
   `settling_a_failure_restores_the_search_and_keeps_its_token` must fail.
3. In `settle`'s `Err` arm, replace it with `{ drop(attested); Restored::Kept }` (settles nothing)
   → `settling_a_failure_the_clerk_has_already_edited_past_still_unlatches_the_store` must fail on
   its final `take`.

Revert each mutation before applying the next. **Use `git diff` to confirm the file is back to the
implementation** — never `git checkout -- <file>`, which discards the whole file's uncommitted
work.

- [ ] **Step 6: Commit**

```bash
cd /Users/hherb/src/cairn-ehr
python3 scripts/check_closing_keywords.py || true
git add cairn-gui/cairn-gui-funnel/src/token.rs
git commit -m "$(cat <<'EOF'
feat(funnel): a registration outcome settles the token store, both ways (#659)

`register` hands the attested search back inside its error because `restore`
needs exactly that value. That made `.map_err(|(e, _)| e)?` — the idiom slice
2c's command handler would have been written with — a silent trap: it drops the
attestation and leaves `in_flight` set, and because `discard` deliberately does
not clear that flag, the clerk editing the form does not recover either.

`TokenStore::settle` commits on Ok, restores on Err, and returns what became of
the search so the window can tell "press Register again" from "wait for the next
search". Generic on T/E because naming `DataError` here would invert the
`cairn-gui-data` -> `cairn-gui-funnel` edge into a cycle.

Deliberately NOT done: clearing `in_flight` in `discard`. That is the other
reading of #659 and it is wrong — it is how two clicks produced two charts.

Three tests, each proven by its own mutation.

Co-Authored-By: Claude Opus 5 (1M context) <noreply@anthropic.com>
EOF
)"
```

---

## Task 2: an injectable one-shot failure on the mock ports (#660)

**Files:**
- Modify: `cairn-gui/cairn-gui-data/src/mock/mod.rs:31-45` (the `MockData` struct and
  `with_fixtures`)
- Modify: `cairn-gui/cairn-gui-data/src/mock/funnel.rs:185-199` (both port impls) and its
  `#[cfg(test)] mod tests`

**Interfaces:**
- Consumes: `TokenStore::settle` from Task 1; the existing `DataError::{Refused, Unavailable}`,
  `MockData::{with_fixtures, search_now, register_now}`.
- Produces: `pub fn MockData::fail_next(&self, e: DataError)` — one shared one-shot slot consumed
  by whichever of the two ports is awaited next. Slice 2c's `--mock` rendering tests are its
  consumers.

**Why `&self` and not `&mut self` as #660 sketches.** Both port methods take `&self`, and
`MockData` already keeps its mutable state behind a `Mutex` for exactly that reason. A
`&mut self` setter would force a 2c test to juggle a mutable binding across a borrow the port
holds; a `&self` setter is consistent with everything else on this type.

**Why one shared slot and not one per port.** The walk that matters — *register fails, the form
keeps its values, the clerk edits and succeeds* — arms the slot once before each call it wants to
fail. Two slots would let a test arm the wrong one and pass for the wrong reason.

- [ ] **Step 1: Write the failing tests**

Append to `mod tests` in `cairn-gui/cairn-gui-data/src/mock/funnel.rs`:

```rust
    // --- #660: `--mock` must be able to fail, or 2c's two sentences ship untested ---

    /// An armed failure is returned instead of the fixture answer, ONCE.
    ///
    /// One-shot rather than sticky: a sticky mock cannot express *"it failed, the clerk fixed
    /// it, it worked"*, which is the only walk that exercises the recovery path at all.
    #[tokio::test]
    async fn an_armed_browse_failure_fires_once_and_then_the_fixtures_come_back() {
        let data = MockData::with_fixtures();
        data.fail_next(DataError::Unavailable("the node was unreachable".to_string()));

        let first = data.search(&SearchQuery::new("mich", None, &[]), TODAY).await;
        assert!(
            matches!(&first, Err(DataError::Unavailable(t)) if t.contains("unreachable")),
            "the armed failure must reach the caller verbatim — a mock that rewrote it would \
             teach 2c's rendering the wrong sentence; got {first:?}"
        );

        assert_eq!(
            found(&browse(&data, "mich").await),
            ["Michaelowski, Samantha"],
            "the NEXT call must answer from fixtures again"
        );
    }

    /// A registration failure hands the attestation back, exactly as the live port does.
    ///
    /// This is the property the whole one-shot exists for: if the mock dropped the
    /// `AttestedSearch` on the failing path, every `--mock` test of the recovery walk would
    /// pass while the real walk latched the store.
    #[tokio::test]
    async fn an_armed_registration_failure_returns_the_attestation_it_was_given() {
        let data = MockData::with_fixtures();
        let mut store = TokenStore::new();
        let query = SearchQuery::new("Nobody Here", Some("1990-01-01"), &[]);
        let token = store
            .record(query, bound_for_prompt(&CandidateList::default()))
            .expect("a non-empty query");
        let attested = store.take(token).expect("the only token");

        data.fail_next(DataError::Refused("the floor said no".to_string()));
        let outcome = data.register(attested, Some("Nobody Here")).await;

        let Err((error, restored)) = store.settle(outcome) else {
            panic!("an armed failure must reach the caller as a failure");
        };
        assert!(matches!(&error, DataError::Refused(t) if t.contains("the floor said no")));
        assert_eq!(
            restored,
            Restored::Kept,
            "settling must put the search back, or the clerk is made to re-search after a \
             failure that changed nothing"
        );
        assert!(
            store.take(token).is_ok(),
            "and the same token must still be redeemable for the retry"
        );
    }

    /// THE WHOLE WALK, in `--mock`, end to end: register fails, the clerk edits, it succeeds.
    ///
    /// The design's *"Register fails. The form keeps its values."* — and the reason #660 asked
    /// for a one-shot. Nothing else in this crate exercises
    /// `record -> take -> settle(Err) -> discard -> record -> take -> settle(Ok)`.
    #[tokio::test]
    async fn a_failed_registration_is_recoverable_by_editing_and_registering_again() {
        let data = MockData::with_fixtures();
        let mut store = TokenStore::new();

        let first = store
            .record(
                SearchQuery::new("Jon Mistyped", Some("1974-05-06"), &[]),
                bound_for_prompt(&CandidateList::default()),
            )
            .expect("a non-empty query");
        let attested = store.take(first).expect("the only token");
        data.fail_next(DataError::Unavailable("a hiccup".to_string()));
        assert!(store.settle(data.register(attested, Some("Jon Mistyped")).await).is_err());

        // The clerk corrects the name; the old search must not license the new registration.
        store.discard();
        let second = store
            .record(
                SearchQuery::new("John Corrected", Some("1974-05-06"), &[]),
                bound_for_prompt(&CandidateList::default()),
            )
            .expect("a non-empty query");
        let retry = store.take(second).expect("the corrected search");
        let id = store
            .settle(data.register(retry, Some("John Corrected")).await)
            .expect("the second attempt is not armed to fail");

        assert_eq!(
            found(&browse(&data, "John Corrected").await),
            ["John Corrected"],
            "the chart the retry created must be findable, and under the CORRECTED name"
        );
        assert!(!id.is_nil());
    }
```

Add whatever `use` lines these need to the test module — `cairn_gui_funnel::Restored` and
`uuid::Uuid` are the likely additions. Read the existing `use` block first; `TokenStore` and
`bound_for_prompt` are already imported there.

**⚠️ `found(&browse(&data, "John Corrected"))` depends on the mock's display-name shape.** Read
`register_now` (in the same file) to see exactly what `display_name` it mints for a registered
patient, and assert that string — do not guess `"Corrected, John"` or `"John Corrected"` from this
plan. If the mock's shape makes the assertion awkward, assert on the candidate count plus the
minted `id` instead, and say why in a comment.

- [ ] **Step 2: Run the tests to verify they fail**

```bash
cd /Users/hherb/src/cairn-ehr/cairn-gui && CAIRN_ALLOW_DB_SKIP=1 cargo test -p cairn-gui-data fail_next
```

Expected: FAIL to compile — `no method named 'fail_next' found for struct 'MockData'`.

- [ ] **Step 3: Add the slot and its setter**

In `cairn-gui/cairn-gui-data/src/mock/mod.rs`, extend the struct and `with_fixtures`:

```rust
pub struct MockData {
    patients: Mutex<Vec<FixturePatient>>,
    note_refs: Vec<NoteRef>,
    /// The failure the NEXT funnel-port call returns instead of its fixture answer. **One-shot**:
    /// taking it clears it.
    ///
    /// # Why a mock that can fail is not a contradiction
    ///
    /// `--mock` is the mode the operator-accessibility pass and the §1.2 timing runbook run in,
    /// and since #648 a failed call is *three* facts rather than two: a floor verdict
    /// (`Refused` — "this cannot succeed as typed") reads differently to the clerk than an
    /// outage (`Unavailable` — "try again"). Those two sentences are the window's, and without
    /// this slot the only coverage for them would be `cairn-gui-live`'s DB-gated suite, which
    /// tests the *classification* and renders nothing at all. See
    /// [#660](https://github.com/cairn-ehr/cairn-ehr/issues/660).
    ///
    /// One slot shared by both ports, not one each: a test arms it immediately before the call
    /// it means to fail, and two slots would let it arm the wrong one and pass for the wrong
    /// reason.
    next_failure: Mutex<Option<DataError>>,
}
```

```rust
    pub fn with_fixtures() -> Self {
        Self {
            patients: Mutex::new(fixtures::starting_population()),
            note_refs: vec![/* unchanged */],
            next_failure: Mutex::new(None),
        }
    }

    /// Arm the next funnel-port call to fail with `e` instead of answering from fixtures.
    ///
    /// `&self`, not `&mut self`: both ports take `&self`, and this type already keeps its
    /// mutable state behind a `Mutex` for that reason. A `&mut self` setter would make a test
    /// juggle a mutable binding across a borrow the port holds.
    pub fn fail_next(&self, e: DataError) {
        *self.next_failure.lock().expect("the armed failure") = Some(e);
    }

    /// Take the armed failure if there is one, clearing it. **One-shot.**
    ///
    /// Private: arming is a test affordance, consuming is the ports' business.
    fn armed_failure(&self) -> Option<DataError> {
        self.next_failure.lock().expect("the armed failure").take()
    }
```

Add `use crate::port::DataError;` to `mod.rs` if it is not already imported.

- [ ] **Step 4: Consume it in both port impls**

In `cairn-gui/cairn-gui-data/src/mock/funnel.rs`, replace the two impls:

```rust
impl PatientSearch for MockData {
    async fn search(&self, query: &SearchQuery, today: &str) -> Result<CandidateList, DataError> {
        // The armed failure is consumed INSIDE the async body, not before it, for the same
        // reason `search_now`/`register_now` exist: these ports must do their work when
        // AWAITED, never when the future is built. A `fail_next` consumed at call time would
        // be spent by a future that was built and dropped.
        match self.armed_failure() {
            Some(e) => Err(e),
            None => Ok(self.search_now(query, today)),
        }
    }
}

impl PatientRegistration for MockData {
    async fn register(
        &self,
        attested: AttestedSearch,
        name: Option<&str>,
    ) -> Result<Uuid, (DataError, AttestedSearch)> {
        // The attestation goes BACK inside the error, exactly as the live port does. A mock
        // that dropped it here would let every `--mock` test of the recovery walk pass while
        // the real walk latched the token store — which is the whole reason #659 and #660 are
        // the same slice.
        match self.armed_failure() {
            Some(e) => Err((e, attested)),
            None => Ok(self.register_now(&attested, name)),
        }
    }
}
```

- [ ] **Step 5: Run the tests to verify they pass**

```bash
cd /Users/hherb/src/cairn-ehr/cairn-gui && CAIRN_ALLOW_DB_SKIP=1 cargo test -p cairn-gui-data
```

Expected: PASS, all three new tests named, nothing else newly red.

- [ ] **Step 6: Prove each test by a mutation**

1. Make `armed_failure` peek instead of take (`.clone()` instead of `.take()`) — the failure
   becomes sticky → `an_armed_browse_failure_fires_once_and_then_the_fixtures_come_back` must fail.
2. In `register`'s failing arm, drop the attestation:
   `Some(e) => { drop(attested); unreachable!() }` — or, to keep it compiling, return a *fresh*
   attestation. Simpler equivalent mutation: change `Err((e, attested))` to call
   `self.register_now(&attested, name)` first and then fail →
   `an_armed_registration_failure_returns_the_attestation_it_was_given` must fail (the mock would
   have minted a chart for a call it reported as failed).
3. Consume the armed failure *outside* the async body (before the `match`, at call time) →
   nothing should break in these three tests, **and that is a finding to record in the commit
   message, not a reason to skip the mutation**: it means the one-shot's await-time semantics are
   unpinned. If that is the outcome, add a fourth test that builds a `register` future, drops it
   without awaiting, and asserts the armed failure is still armed.

Revert each mutation with an edit, verified by `git diff`.

- [ ] **Step 7: Commit**

```bash
cd /Users/hherb/src/cairn-ehr
python3 scripts/check_closing_keywords.py || true
git add cairn-gui/cairn-gui-data/src/mock/
git commit -m "$(cat <<'EOF'
feat(funnel): --mock can fail, so 2c's two sentences can be tested (#660)

Since #648 a failed call is three facts: a floor verdict reads differently to
the clerk than an outage. Those two sentences are slice 2c's to write, and
`--mock` -- the mode the accessibility pass and the timing runbook run in --
could not produce either, so the rendering would have shipped with coverage only
from a DB-gated suite that renders nothing.

`MockData::fail_next` arms one shared one-shot slot, consumed by whichever port
is awaited next. One-shot rather than sticky: a sticky mock cannot express "it
failed, the clerk fixed it, it worked", which is the only walk that exercises
recovery. Consumed inside the async body, not at call time, for the same reason
`search_now`/`register_now` exist.

The failing `register` arm hands the attestation back exactly as the live port
does -- a mock that dropped it would let every --mock recovery test pass while
the real walk latched the store.

Co-Authored-By: Claude Opus 5 (1M context) <noreply@anthropic.com>
EOF
)"
```

---

## Task 3: a deterministic Rust-side refusal stops reading as an outage (#651)

**Files:**
- Modify: `crates/cairn-node/src/db_diagnosis.rs` (add the type and its two functions beside
  `LocalDbFault` at `:280`)
- Modify: `crates/cairn-node/src/patient/register.rs:131-145` (`dob_precision`)
- Modify: `cairn-gui/cairn-gui-live/src/error.rs` (`data_error_from` at `:122`, and the module doc
  that describes the gap)
- Modify: `cairn-gui/cairn-gui-live/tests/refusal_is_not_an_outage.rs:145-183` (the pinned test)

**Interfaces:**
- Consumes: the existing `anyhow` context chains and
  `cairn_gui_live::error::{refusal_is_deliberate, sqlstate_of}`.
- Produces, in `cairn_node::db_diagnosis`:
  ```rust
  pub struct DeliberateRefusal { /* private */ }
  pub fn deliberate_refusal(message: impl Into<String>) -> anyhow::Error;
  pub fn is_deliberate_refusal(e: &anyhow::Error) -> bool;
  ```
  `cairn-gui-live`'s `data_error_from` is the first consumer; #652's consolidation is expected to
  gather the P0001 rule into the same module.

**Which of #651's two shapes this takes, and why.** The issue offers (1) a typed error, or (2) a
marker layer on the `anyhow` chain. This is **(1)** — a real type with a private field, constructed
only through `deliberate_refusal`. The issue's own objection to (2) is decisive: it makes *"is this
a verdict"* a convention again, which is precisely what #648 was trying to get away from.

**Home:** `db_diagnosis`, because #651 says it *"wants the same home as the consolidation of the
P0001 rule itself"* and #652 names `cairn_node::db_diagnosis` as that home. Putting it there now
means #652 finds the module already answering both halves of the question.

**⚠️ Scope discipline.** Convert **`dob_precision` only** in this task. Grep
`crates/cairn-node/src/patient/register.rs` for other `bail!`/`anyhow!` pre-flight refusals; if you
find any, **list them in the commit message and leave them**, because each one is a judgement about
whether that particular failure is deterministic, and a sweep is a different slice's blast radius.

- [ ] **Step 1: Write the failing unit tests in `db_diagnosis.rs`**

Append to that file's existing `#[cfg(test)] mod tests`:

```rust
    // --- #651: a refusal raised in Rust is as much a verdict as one raised in the floor ---

    /// The marker survives the `.context(…)` layers every orchestrator adds.
    ///
    /// This is the whole reason it walks the chain rather than reading the outermost error:
    /// `register_patient` wraps its failures in context naming the operation, so a check that
    /// looked only at the top would answer `false` for every real refusal.
    #[test]
    fn a_deliberate_refusal_is_recognised_through_a_context_chain() {
        let e = deliberate_refusal("birth date \"3/2/1980\" is not a recognised shape")
            .context("asserting the date of birth")
            .context("registering the patient");
        assert!(
            is_deliberate_refusal(&e),
            "a verdict that stops being recognisable once an orchestrator adds context is a \
             verdict nobody can act on"
        );
    }

    /// An ordinary failure is NOT one, and that direction is the dangerous one to get wrong.
    ///
    /// Calling an outage a refusal tells a clerk to change a form that was never the problem.
    #[test]
    fn an_ordinary_failure_is_not_a_deliberate_refusal() {
        let e = anyhow::anyhow!("connection closed").context("registering the patient");
        assert!(!is_deliberate_refusal(&e));
    }

    /// The message reaches the operator unchanged.
    ///
    /// §9.6: an in-DB floor refusal is legible on purpose, and the text is the only thing that
    /// tells the clerk what to change. A Rust-side refusal is held to the same standard, so
    /// `operator_chain` must still render its sentence.
    #[test]
    fn a_deliberate_refusals_own_sentence_survives_into_the_operator_rendering() {
        let e = deliberate_refusal("birth date \"3/2/1980\" is not a recognised shape")
            .context("registering the patient");
        assert!(operator_chain(&e).contains("not a recognised shape"));
    }
```

Add `use anyhow::Context as _;` to that test module if it is not already there.

- [ ] **Step 2: Run them to verify they fail**

```bash
cd /Users/hherb/src/cairn-ehr && cargo test -p cairn-node --lib db_diagnosis
```

Expected: FAIL to compile — `cannot find function 'deliberate_refusal' in this scope`.

- [ ] **Step 3: Implement the type**

Add to `crates/cairn-node/src/db_diagnosis.rs`, beside `LocalDbFault`:

```rust
/// A refusal this node raised **deliberately, in Rust, before any statement reached Postgres**.
///
/// # The question this answers
///
/// Every caller that renders a failure to a human has to decide one thing: was this a *verdict*
/// about the call, or an *accident* that befell it? Get it backwards and the harm is real in
/// both directions — calling an outage a refusal tells a clerk to change a form that was never
/// the problem, and calling a refusal an outage hands them a retry button for a verdict, which
/// they will press.
///
/// For refusals the in-DB floor raises there is already an answer: a bare `RAISE EXCEPTION` is
/// SQLSTATE `P0001`, which `db/001_envelope.sql` states is a contract and
/// `crates/cairn-node/tests/floor_refusals_carry_no_errcode.rs` enforces across every
/// `db/*.sql`.
///
/// That answer is unavailable for a refusal raised **above** the database. `register_patient`
/// validates the shape of a date of birth up front, deliberately, so a malformed one refuses the
/// whole call with zero side effects — no HLC tick, no partial chart (#350). That refusal is
/// every bit as deterministic as the floor's: the same string refuses identically forever. But
/// it carried no SQLSTATE anywhere in its chain, so it was **indistinguishable from a dropped
/// connection**, and the clerk most likely to meet it is the one on a desk with no date widget
/// who typed `3/2/1980`. See
/// [#651](https://github.com/cairn-ehr/cairn-ehr/issues/651).
///
/// # Why a type and not a convention
///
/// The alternative considered in #651 was a marker string or a sentinel on the chain. A type
/// with a private field, constructible only through [`deliberate_refusal`], means *"is this a
/// verdict"* is answered by the compiler rather than by a convention — which is what #648 was
/// trying to get away from in the first place.
///
/// # What it is NOT
///
/// It is not a claim that every failure carrying it is *safe*, and it is not a second error
/// vocabulary. It is one bit — *this node decided this, and deciding it again will decide the
/// same* — carried alongside a message that is already written for a human.
#[derive(Debug)]
pub struct DeliberateRefusal {
    message: String,
}

impl std::fmt::Display for DeliberateRefusal {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.message)
    }
}

impl std::error::Error for DeliberateRefusal {}

/// Build a deterministic refusal as an `anyhow::Error`, ready to be `?`'d and contextualised.
///
/// Use this **only** where the refusal is genuinely deterministic — the same inputs refuse the
/// same way forever, and nothing about the environment (a connection, a lock, a disk) took part
/// in the decision. A retry must be pointless by construction; if a retry might work, this is
/// the wrong constructor and the honest answer is an ordinary error.
pub fn deliberate_refusal(message: impl Into<String>) -> anyhow::Error {
    anyhow::Error::new(DeliberateRefusal {
        message: message.into(),
    })
}

/// Did this node deliberately refuse, above the database?
///
/// Walks the **whole** `anyhow` chain, for the same reason [`operator_chain`] and
/// `cairn-gui-live`'s `sqlstate_of` do: every orchestrator adds `.context("…")` layers naming the
/// operation, so a check that read only the outermost error would answer `false` for every
/// refusal a real call site produces.
///
/// A `false` here means only *"not one of these"*. The caller must still ask the SQLSTATE
/// question about floor refusals — the two discriminators are complementary, not alternatives.
pub fn is_deliberate_refusal(e: &anyhow::Error) -> bool {
    e.chain()
        .any(|cause| cause.downcast_ref::<DeliberateRefusal>().is_some())
}
```

- [ ] **Step 4: Run to verify the unit tests pass**

```bash
cd /Users/hherb/src/cairn-ehr && cargo test -p cairn-node --lib db_diagnosis
```

Expected: PASS.

- [ ] **Step 5: Make `dob_precision` refuse deliberately**

In `crates/cairn-node/src/patient/register.rs`, replace the `anyhow::bail!` at `:139-145`:

```rust
        _ => Err(crate::db_diagnosis::deliberate_refusal(format!(
            "birth date {value:?} is not a recognised shape (expected YYYY, YYYY-MM, or \
             YYYY-MM-DD) — refusing rather than asserting a precision nobody actually gave"
        ))),
```

Keep the message **byte-identical** to what it is today: `refusal_is_not_an_outage.rs` asserts on
`"not a recognised shape"`, and `dob_precision_refuses_an_unrecognised_shape_rather_than_guessing`
in the same file asserts on the refusal. Read the current text before editing and diff it after.

Add to `dob_precision`'s doc comment, above the existing text:

```rust
/// Its refusal is a [`crate::db_diagnosis::DeliberateRefusal`], not a bare `anyhow!`, and that
/// matters to exactly one caller today: `cairn-gui-live` maps a `cairn-node` failure onto the
/// window's error type, and without the marker this refusal — as deterministic as any the floor
/// raises — reached the clerk as an outage with a retry button that could never work (#651).
```

- [ ] **Step 6: Run the node's own register tests**

```bash
cd /Users/hherb/src/cairn-ehr && cargo test -p cairn-node --lib patient::register
```

Expected: PASS — `bail!` and an explicit `Err` are the same value to every existing assertion, so
nothing here should change. If something fails, it is asserting on the error's *type* and wants
reading, not silencing.

- [ ] **Step 7: Teach the GUI's classifier the second discriminator**

In `cairn-gui/cairn-gui-live/src/error.rs`, change `data_error_from`:

```rust
pub fn data_error_from(e: &anyhow::Error) -> DataError {
    let text = cairn_node::db_diagnosis::operator_chain(e);
    // TWO discriminators, complementary and not alternatives. The floor's own refusals carry
    // `P0001`; a refusal `cairn-node` raised in Rust before any statement reached Postgres
    // carries no SQLSTATE at all and is marked instead (#651). Either one means a verdict.
    if refusal_is_deliberate(sqlstate_of(e)) || cairn_node::db_diagnosis::is_deliberate_refusal(e) {
        DataError::Refused(text)
    } else {
        DataError::Unavailable(text)
    }
}
```

Then update the module doc: the section headed **`# What this rule does NOT cover`** now describes
something that *is* covered. Rewrite it to say what the rule covers and how — two discriminators,
one owned by `db/*.sql`'s no-`USING ERRCODE` contract and one owned by `DeliberateRefusal` — and
**keep the #655 paragraph unchanged**, because the `false` half (class 23, `42501`, `42P01`) is
still wrong and is still filed.

- [ ] **Step 8: Flip the pinned test**

In `cairn-gui/cairn-gui-live/tests/refusal_is_not_an_outage.rs`, rename
`a_rust_side_pre_flight_refusal_is_not_yet_told_apart` to
`a_rust_side_pre_flight_refusal_is_refused_not_unavailable` and replace its final assertion block:

```rust
    let DataError::Refused(text) = &err else {
        panic!(
            "a malformed date of birth is a VERDICT: the same string refuses identically \
             forever, and offering a retry for it is the harm #648 describes one layer above \
             where #648 was looking (#651). Got {err:?}"
        );
    };
    assert!(
        text.contains("not a recognised shape"),
        "the orchestrator's own message is the only thing that tells the clerk what to change \
         — got: {text}"
    );
```

Rewrite the test's doc comment: it currently explains that it pins *today's wrong behaviour on
purpose* and instructs a future reader to flip it. That instruction has now been followed, so the
doc should instead record **what the test proves and what mutation kills it** — that removing the
`is_deliberate_refusal` arm from `data_error_from` turns this back into `Unavailable`.

Also update the module doc at the head of that file and `cairn-gui-live/src/lib.rs` if either
describes #651 as open — grep for `651` across `cairn-gui/` and fix every prose reference.

- [ ] **Step 9: Run both trees**

```bash
cd /Users/hherb/src/cairn-ehr && cargo test --workspace 2>&1 | tail -40
```

**Do not pipe the gate you rely on** — that is only to skim. The gate that counts is run without a
pipe so cargo's exit code is visible:

```bash
cd /Users/hherb/src/cairn-ehr && cargo test -p cairn-node -p cairn-sync
```

Then the GUI tree's DB-gated suite, which is the one that actually proves this (see
`docs/HANDOVER.md` for the connection string; the suite skips itself without one):

```bash
cd /Users/hherb/src/cairn-ehr/cairn-gui && cargo test -p cairn-gui-live --test refusal_is_not_an_outage
```

Expected: PASS, including `a_rust_side_pre_flight_refusal_is_refused_not_unavailable`. **If it
skips, the DB is not configured and this task is not verified** — say so rather than reporting
green.

- [ ] **Step 10: Prove it by a mutation**

Remove `|| cairn_node::db_diagnosis::is_deliberate_refusal(e)` from `data_error_from` and re-run
the DB-gated suite: `a_rust_side_pre_flight_refusal_is_refused_not_unavailable` must fail. Restore
it, verified by `git diff`.

- [ ] **Step 11: Commit**

```bash
cd /Users/hherb/src/cairn-ehr
python3 scripts/check_closing_keywords.py || true
git add crates/cairn-node/src/db_diagnosis.rs crates/cairn-node/src/patient/register.rs \
        cairn-gui/cairn-gui-live/src/error.rs cairn-gui/cairn-gui-live/src/lib.rs \
        cairn-gui/cairn-gui-live/tests/refusal_is_not_an_outage.rs
git commit -m "$(cat <<'EOF'
fix(#651): a refusal raised in Rust is a verdict, not an outage

`register_patient` validates the shape of a date of birth before ticking any
HLC, deliberately, so a malformed one refuses with zero side effects. That
refusal is as deterministic as any the floor raises -- the same string refuses
identically forever -- but it carried no SQLSTATE, so `data_error_from` could
only call it `Unavailable` and hand the clerk a retry button that can never
work. On a desk with no date widget, `3/2/1980` is the DEFAULT failure mode: it
searches fine, finds nothing, then fails in Rust.

`cairn_node::db_diagnosis::DeliberateRefusal` is the typed answer -- #651's
shape 1, not the marker-convention shape 2, because a convention is the thing
#648 was trying to get away from. It walks the whole anyhow chain, because every
orchestrator adds context layers. Its home is `db_diagnosis` so that #652's
consolidation finds the module already answering both halves.

The test that pinned the wrong behaviour on purpose now expects `Refused`.

Converted `dob_precision` only. Other pre-flight refusals in the tree are each a
judgement about whether that failure is deterministic, and a sweep is a
different blast radius.

Co-Authored-By: Claude Opus 5 (1M context) <noreply@anthropic.com>
EOF
)"
```

---

## Task 4: one enrolment rule — no surface provisions on a write path (#654)

**Files:**
- Create: `crates/cairn-node/src/actor_enrolment.rs`
- Modify: `crates/cairn-node/src/lib.rs` (declare the module)
- Modify: `crates/cairn-node/src/main.rs` — delete `ensure_registration_actor` (`:5437`), add the
  `EnrollDeviceActor` subcommand, call the enrolment from `Cmd::Init` (`:2184`), and replace the
  fifteen call sites listed below
- Create: `crates/cairn-node/tests/device_actor_enrolment.rs`

**The fifteen call sites, by line as of `main.rs` today.** Re-grep rather than trusting these
numbers — earlier edits in this task move them:

| Line | Subcommand |
|---|---|
| 3883 | `RegisterJohnDoe` |
| 3959 | `PatientRegister` |
| 4029 | `SensitivityAssert` |
| 4336 | `AssertObservedEvidence` |
| 4397 | `AssertIdentityEvidence` |
| 4480 | `IdentifyPatient` |
| 4546 | `MedicationAssert` |
| 4592 | `MedicationCease` |
| 4632 | `MedicationChangeDose` |
| 4678 | `MedicationCorrectDose` |
| 4724 | `MedicationCode` |
| 4769 | `MedicationCodeCorrect` |
| 4814 | `MedicationReconcile` |
| 4852 | `MedicationSeparate` |
| 5194 | `Shred` |

**Interfaces:**
- Consumes: `DeliberateRefusal`/`deliberate_refusal` from Task 3 — the refusal is a verdict, and
  making it one is how the GUI will eventually classify it correctly too.
- Produces, in `cairn_node::actor_enrolment`:
  ```rust
  pub async fn device_actor_enrolled(db: &tokio_postgres::Client, kid: &str) -> anyhow::Result<bool>;
  pub async fn enroll_device_actor(db: &tokio_postgres::Client, kid: &str) -> anyhow::Result<bool>;
  pub async fn require_device_actor(db: &tokio_postgres::Client, kid: &str) -> anyhow::Result<()>;
  pub fn not_enrolled_refusal(kid: &str) -> anyhow::Error;
  ```
  `main.rs` consumes all four. Slice 2c's `build_live_state` is expected to consume
  `device_actor_enrolled` for its launch-time probe.

**⚠️ The property that must survive the move, and it is the whole reason the old function was
written the way it was.** The existence check is **kind-agnostic** — `WHERE signing_key_id = $1`
with no `AND kind = 'device'`. `submit_event` resolves a signer to an actor purely by
`signing_key_id`, and if one key maps to **more than one** `actor_current` row it sets
`actor_id = NULL` for *every* event that key authors node-wide (db/005,
`array_length(v_actor_ids, 1) = 1`), silently and irreversibly degrading attribution. A
kind-scoped guard would happily add a second actor to a key already enrolled as a matcher `agent`
or a `human`. **Copy the original doc comment's argument across verbatim; do not paraphrase it.**

- [ ] **Step 1: Write the failing tests**

Create `crates/cairn-node/tests/device_actor_enrolment.rs`. This tree's DB-gating idiom is
`mod common;` plus `common::cs()` — which is `std::env::var("CAIRN_TEST_PG").ok()`, so a suite
skips itself by returning early when it is `None` — and `common::setup(&client, &extra_tables)`,
which truncates the clinical core and **enrols one `agent` signer**, returning
`(SigningKey, String)`. See `crates/cairn-node/tests/patient_register_demographics.rs` for a
worked example. Do not invent a second idiom.

**`setup` enrolling an `agent` is a gift for the hardest test here.** The dual-mapping case needs
a key already enrolled under a kind that is *not* `device`, and `setup` hands one back. So
`enrol_as_some_other_kind` is not a helper this file has to write at all — it is
`common::setup(&db, &[]).1`.

**A consequence worth knowing before you start Step 8:** every existing DB-gated suite in this
directory calls `register_patient` and friends as **library** functions, which never called
`ensure_registration_actor`, and gets an enrolled key from `setup` anyway. So Task 4's blast radius
on this directory is far smaller than the fifteen call sites suggest — the sites are in `main.rs`,
and only a test that *spawns the binary* can notice. `crates/cairn-node/tests/cli_sensitivity_surface.rs`
is the one such suite.

```rust
//! The one enrolment rule (#654): a device actor is PROVISIONED, never minted on a write path.
//!
//! Before this, `cairn-node`'s fifteen write subcommands each enrolled the node's signing key as
//! a `device` actor on first use, while `cairn-gui-live` deliberately did not — so a node's
//! behaviour depended on which surface touched it first, and the reference window's first
//! registration refused with a message naming a key rather than a remedy.

mod common; // if this tree's test helpers live there; otherwise inline the connect helper

use cairn_node::actor_enrolment::{
    device_actor_enrolled, enroll_device_actor, not_enrolled_refusal, require_device_actor,
};
use cairn_node::db_diagnosis::is_deliberate_refusal;

/// The refusal names the COMMAND, not just the key.
///
/// Pure, so it runs with no database. This is the sentence's whole job: `submit_event`'s own
/// refusal — *"signer 9f3c… is not an enrolled, non-revoked actor"* — is true, legible, and
/// tells nobody what to do about it. The precedent is `submit_event`'s unwrap-key refusal, which
/// names `establish-unwrap-key`.
#[test]
fn the_refusal_names_the_command_that_fixes_it() {
    let e = not_enrolled_refusal("9f3cdeadbeef");
    let rendered = format!("{e:#}");
    assert!(
        rendered.contains("enroll-device-actor"),
        "a refusal that does not name its remedy leaves the operator exactly where the floor's \
         own message left them — got: {rendered}"
    );
    assert!(
        rendered.contains("9f3cdeadbeef"),
        "and it must still name the key, so an operator with several can tell which — got: \
         {rendered}"
    );
}

/// It is a VERDICT, so a window can classify it without reaching into the database.
///
/// The same discriminator #651 added. A pre-flight refusal that reached a clerk as an outage
/// would offer a retry that can never work.
#[test]
fn the_refusal_is_a_deliberate_verdict_not_an_accident() {
    assert!(is_deliberate_refusal(&not_enrolled_refusal("9f3cdeadbeef")));
}

/// On a node nobody provisioned, the requirement REFUSES rather than provisioning.
#[tokio::test]
async fn an_unprovisioned_node_refuses_rather_than_enrolling_on_the_write_path() {
    let Some(db) = connect_or_skip().await else { return };
    let kid = a_key_id("unprovisioned");

    assert!(!device_actor_enrolled(&db, &kid).await.unwrap());
    let e = require_device_actor(&db, &kid)
        .await
        .expect_err("a write path must never provision");
    assert!(is_deliberate_refusal(&e));
    assert!(
        !device_actor_enrolled(&db, &kid).await.unwrap(),
        "REFUSING MUST NOT ENROL. A check with a side effect is the thing this issue is about \
         — trap 2's shape, one subsystem over"
    );
}

/// Enrolling is idempotent, and says whether it did anything.
#[tokio::test]
async fn enrolling_twice_enrols_once_and_reports_it() {
    let Some(db) = connect_or_skip().await else { return };
    let kid = a_key_id("idempotent");

    assert!(enroll_device_actor(&db, &kid).await.unwrap(), "the first call enrols");
    assert!(!enroll_device_actor(&db, &kid).await.unwrap(), "the second finds it already there");
    assert_eq!(rows_for(&db, &kid).await, 1);
    require_device_actor(&db, &kid).await.expect("now it passes");
}

/// ⚠️ THE PROPERTY THE WHOLE DESIGN TURNS ON: the existence check is KIND-AGNOSTIC.
///
/// `submit_event` resolves a signer to an actor purely by `signing_key_id`. If one key maps to
/// MORE than one `actor_current` row, db/005 sets `actor_id = NULL` for EVERY event that key
/// authors node-wide (`array_length(v_actor_ids, 1) = 1`) — silently and irreversibly degrading
/// attribution. So a key already enrolled as (say) a matcher `agent` or a `human` must be left
/// ALONE, not given a second `device` row.
///
/// A `kind = 'device'`-scoped check would pass every other test in this file and cause exactly
/// that.
#[tokio::test]
async fn a_key_already_enrolled_under_another_kind_is_left_alone() {
    let Some(db) = connect_or_skip().await else { return };
    let kid = a_key_id("already-an-agent");
    enrol_as_some_other_kind(&db, &kid).await;

    assert!(
        device_actor_enrolled(&db, &kid).await.unwrap(),
        "already-authoring means already enrolled, whatever kind it wears"
    );
    assert!(!enroll_device_actor(&db, &kid).await.unwrap());
    assert_eq!(
        rows_for(&db, &kid).await,
        1,
        "a SECOND actor for one key nulls the actor_id of every event that key ever authors"
    );
}
```

Write the four helpers below the tests. `a_key_id` must **derive** its bytes at runtime, never
write a literal — and it is a `lineage`, not a `salt`:

```rust
/// A distinct, deterministic key id per test, derived rather than written.
///
/// Derived because a byte-array literal in a crypto-adjacent context trips CodeQL's
/// `rust/hard-coded-cryptographic-value`; named `lineage` rather than `salt`/`nonce`/`seed`
/// because CodeQL picks its sink by the NAME of the binding a value flows into, and this
/// discriminates test rows — it constructs nothing cryptographic. See `CLAUDE.md` rule 6 and
/// `crates/cairn-node/tests/crypto_sink_names_are_genuine.rs`.
fn a_key_id(lineage: &str) -> String {
    let bytes: [u8; 32] = std::array::from_fn(|i| {
        (i as u8).wrapping_mul(7).wrapping_add(lineage.len() as u8).wrapping_add(
            lineage.as_bytes().get(i % lineage.len().max(1)).copied().unwrap_or(0),
        )
    });
    hex::encode(bytes)
}

/// How many `actor_current` rows this key maps to. ONE is the only safe answer.
async fn rows_for(db: &tokio_postgres::Client, kid: &str) -> i64 {
    db.query_one(
        "SELECT count(*) FROM actor_current WHERE signing_key_id = $1",
        &[&kid],
    )
    .await
    .expect("count the actors this key maps to")
    .get(0)
}
```

The remaining two helpers are one line each, because `common` already has them:

```rust
/// Connect, or skip: `cs()` is `CAIRN_TEST_PG`, and its absence is how every DB-gated suite in
/// this directory skips itself rather than failing on a machine with no Postgres.
async fn connect_or_skip() -> Option<tokio_postgres::Client> {
    let cs = common::cs()?;
    Some(common::connect(&cs).await)   // use this directory's own connect helper — read one suite
}

/// A key already enrolled under a kind that is NOT `device`.
///
/// `common::setup` enrols an `agent` and hands its key back, which is exactly the fixture the
/// dual-mapping test needs — an `agent` with a model/skill-epoch blob is what a matcher is, and
/// giving it a second `device` row is the failure being guarded against.
async fn enrol_as_some_other_kind(db: &tokio_postgres::Client) -> String {
    common::setup(db, &[]).await.1
}
```

The dual-mapping test then reads `let kid = enrol_as_some_other_kind(&db).await;` and does **not**
call `a_key_id` — using `setup`'s own key is what makes the fixture a real second-kind enrolment
rather than a hand-composed one. Adjust `a_key_already_enrolled_under_another_kind_is_left_alone`
accordingly when you write it.

⚠️ `common::setup` **truncates** `event_log`, `actor_event` and the clinical core. Call it first in
any test that uses it, and do not call it in the middle of a test that has already enrolled
something — it would wipe it. The three DB-gated tests here each own their connection, so ordering
within a test is the only concern.

- [ ] **Step 2: Run to verify they fail**

```bash
cd /Users/hherb/src/cairn-ehr && cargo test -p cairn-node --test device_actor_enrolment
```

Expected: FAIL to compile — `unresolved import 'cairn_node::actor_enrolment'`.

- [ ] **Step 3: Create the module**

Create `crates/cairn-node/src/actor_enrolment.rs`. Move the kind-agnostic argument across from
`main.rs:5420-5436` **verbatim** — it is the most load-bearing prose in this task:

```rust
//! The one enrolment rule: a node's signing key becomes an authoring actor by **provisioning**,
//! never as a side effect of a write.
//!
//! # Why this module exists at all
//!
//! Until #654, `main.rs` carried `ensure_registration_actor`, called by fifteen write
//! subcommands, which enrolled the node's key as a `device` actor on first use. The reference
//! window's `cairn-gui-live` deliberately did **not**, because provisioning on a write path is
//! the shape ADR-0066 decision 6 forbids for the unwrap key (`ensure_unwrap_key`/`submit_event`
//! were made to *refuse* rather than quietly provision — trap 2). The result was an asymmetry
//! in which a node's behaviour depended on which surface touched it first, and the window's very
//! first registration refused with a message naming a key rather than a remedy.
//!
//! One rule now: **nothing provisions on a write path.** `cairn-node init` enrols, so an
//! initialised node costs its operator no new act. `cairn-node enroll-device-actor` is the named
//! remedy for a node that was not — a node restored without its actor registry, most obviously,
//! since a restore never runs `init`. Every write path *requires* and refuses.
//!
//! # What an enrolled device actor is, and is not
//!
//! It is the headless-node/CLI convenience: the node's own key, enrolled as a `device` with role
//! `registration-desk`, so an unattended node can author. **A real clinical UI attaches the
//! operating clerk's *human* actor instead** — that is principle 10's separable accountability,
//! and this device-key path does not stand in for it.

use anyhow::Context as _;

/// The role recorded in a device actor's pinned determinants.
///
/// Unchanged from the pre-#654 spelling on purpose: it is written into a signed actor event, and
/// renaming it would change bytes on the wire to make a command name read better.
const DEVICE_ACTOR_ROLE: &str = "registration-desk";

/// Is this signing key already resolvable to an authoring actor?
///
/// # ⚠️ KIND-AGNOSTIC, AND THAT IS NOT AN OVERSIGHT
///
/// `submit_event` resolves a signer to an actor purely by `signing_key_id` — kind matters only
/// for attestation — and if one key maps to MORE than one `actor_current` row it sets
/// `actor_id = NULL` for EVERY event that key authors node-wide (db/005,
/// `array_length(v_actor_ids, 1) = 1`), silently and irreversibly degrading attribution.
///
/// A kind-scoped `AND kind = 'device'` guard would happily add a second actor to a key already
/// enrolled as (say) a matcher `agent` or a `human`, tripping exactly that dual-mapping. Keying
/// on `signing_key_id` alone means a key already usable for authoring is left untouched — never
/// split into two actors.
///
/// Public because slice 2c's window probes it **at launch**, the same discipline
/// `build_live_state` already follows by loading the node key up front rather than discovering
/// at sign-off that it can never seal anything.
pub async fn device_actor_enrolled(db: &tokio_postgres::Client, kid: &str) -> anyhow::Result<bool> {
    let enrolled: bool = db
        .query_one(
            "SELECT EXISTS(SELECT 1 FROM actor_current WHERE signing_key_id = $1)",
            &[&kid],
        )
        .await
        .context("checking whether this node's key is an enrolled actor")?
        .get(0);
    Ok(enrolled)
}

/// Provision this key as a `device` actor. Idempotent. Returns whether it actually enrolled.
///
/// An **owner ceremony**: the runtime `cairn_agent` role deliberately cannot `enroll_actor`, so
/// this runs as the operator, from `init` or from `enroll-device-actor`, and never from a write
/// path.
pub async fn enroll_device_actor(db: &tokio_postgres::Client, kid: &str) -> anyhow::Result<bool> {
    if device_actor_enrolled(db, kid).await? {
        return Ok(false);
    }
    let pinned = serde_json::json!({ "role": DEVICE_ACTOR_ROLE, "node_key": kid }).to_string();
    db.execute(
        "SELECT enroll_actor('device', $1::text::jsonb, $2)",
        &[&pinned, &kid],
    )
    .await
    .context("enrolling this node's key as a device actor")?;
    Ok(true)
}

/// The refusal a write path gives on an unprovisioned node.
///
/// A [`crate::db_diagnosis::DeliberateRefusal`] (#651), because it is a verdict: the same key
/// refuses identically until somebody enrols it, and a caller offering a retry would be offering
/// one that can never work.
///
/// It names the **command**, not only the key. `submit_event`'s own refusal — *"signer 9f3c… is
/// not an enrolled, non-revoked actor"* — is true and legible and tells nobody what to do; the
/// precedent for naming the remedy is `submit_event`'s unwrap-key refusal, which names
/// `establish-unwrap-key`.
pub fn not_enrolled_refusal(kid: &str) -> anyhow::Error {
    crate::db_diagnosis::deliberate_refusal(format!(
        "this node's signing key {kid} is not enrolled as an actor, so it may not author \
         clinical events. Enrolling is provisioning, not a side effect of writing: run \
         `cairn-node enroll-device-actor` once on this node. (A node created by `cairn-node \
         init` is already enrolled; a node restored without its actor registry is not.)"
    ))
}

/// Require an enrolled actor before a write, refusing if there is none. **Never provisions.**
pub async fn require_device_actor(db: &tokio_postgres::Client, kid: &str) -> anyhow::Result<()> {
    if device_actor_enrolled(db, kid).await? {
        return Ok(());
    }
    Err(not_enrolled_refusal(kid))
}
```

Declare it in `crates/cairn-node/src/lib.rs` beside the other modules:

```rust
pub mod actor_enrolment;
```

- [ ] **Step 4: Run the new test file**

```bash
cd /Users/hherb/src/cairn-ehr && cargo test -p cairn-node --test device_actor_enrolment
```

Expected: PASS, with all five tests named. If the DB-gated three skip, the two pure ones must
still pass — and **say in your report that three skipped**, never that the task is verified.

- [ ] **Step 5: Wire it into `main.rs` — the subcommand**

Add to the `Cmd` enum, near the other provisioning commands (`Init`, `EstablishUnwrapKey`):

```rust
    /// Enrol this node's signing key as a `device` actor, so it may author clinical events.
    ///
    /// Idempotent, and a one-time act. `init` already does this, so an ordinary node never
    /// needs it; a node restored without its actor registry does, and every write refuses
    /// naming this command until it is run (#654).
    EnrollDeviceActor,
```

And its arm, following the shape of the neighbouring provisioning arms (read one first — they load
the keystore and the schema in an established order):

```rust
        Cmd::EnrollDeviceActor => {
            // The same two lines every write arm opens with (see `Cmd::PatientRegister`):
            // `true` means interactive, so an operator may be prompted to unseal. That is
            // correct here — enrolment is an owner ceremony, and the runtime `cairn_agent`
            // role deliberately cannot `enroll_actor`.
            let sk = load_signing_key(&cli.key, true)?;
            let kid = hex::encode(sk.verifying_key().to_bytes());
            let db = cairn_node::db::connect_and_load_schema(&cli.conn).await?;
            if cairn_node::actor_enrolment::enroll_device_actor(&db, &kid).await? {
                println!("enrolled this node's key {kid} as a device actor");
            } else {
                println!("this node's key {kid} is already an enrolled actor — nothing to do");
            }
        }
```

`connect_and_load_schema` rather than `connect`: this is a provisioning command, and the
neighbouring provisioning arms (`Init`) load the schema. The write arms use bare `connect` because
by then the schema is a precondition.

- [ ] **Step 6: Wire it into `init`**

In `Cmd::Init`, immediately after `cairn_node::identity::provision(...)` (`main.rs:2258`), before
the `println!` that reports the node id:

```rust
            // Provision the authoring actor here, while this is unambiguously an owner ceremony
            // and the key is in hand. Doing it here is what makes `M = 0` for an ordinary
            // operator (#654's paper-parity): an initialised node can author immediately, and
            // no write path has to provision to make that true.
            cairn_node::actor_enrolment::enroll_device_actor(&db, &kid).await?;
```

- [ ] **Step 7: Replace the fifteen call sites**

At each of the fifteen, replace:

```rust
            ensure_registration_actor(&db, &kid).await?;
```

with:

```rust
            cairn_node::actor_enrolment::require_device_actor(&db, &kid).await?;
```

Mind the two spellings — some sites bind `kid`, some bind `node_kid`. Then **delete
`ensure_registration_actor` entirely** (`main.rs:5420-5454`, doc comment included; its argument
now lives in `device_actor_enrolled`'s doc).

Verify nothing references it:

```bash
cd /Users/hherb/src/cairn-ehr && grep -rn 'ensure_registration_actor' crates cairn-gui docs --include='*.rs' --include='*.md' | grep -v '/target/'
```

Expected: only prose references remain. Fix each — `crates/cairn-node/tests/photo_evidence.rs:32`,
`cairn-gui/cairn-gui-live/tests/common/mod.rs:124` and `cairn-gui/cairn-gui-live/src/lib.rs:90`
all describe the old behaviour and are now wrong.

- [ ] **Step 8: Fix every test and script that relied on the side effect**

```bash
cd /Users/hherb/src/cairn-ehr && cargo test --workspace --no-run
```

Then run the suites that shell out to the CLI or drive these subcommands:

```bash
cd /Users/hherb/src/cairn-ehr && cargo test -p cairn-node --test cli_sensitivity_surface
cd /Users/hherb/src/cairn-ehr && cargo test -p cairn-node --test patient_register_demographics \
    --test john_doe --test medication_read --test sensitivity_ceremony --test seal_submit
```

Any fixture that ran a write command against a node it never provisioned now needs
`enroll-device-actor` (or `enroll_device_actor`) first. **Add the provisioning step; never weaken
`require_device_actor` to make a red fixture green** — that is trap 2's instruction, one subsystem
over, and it is the exact move that would undo this task.

Also check `scripts/measure_dr_restore.py` and
`crates/cairn-node/examples/seed_measurement_corpus.rs`, both of which drive write commands.

- [ ] **Step 9: Run the full root gate**

```bash
cd /Users/hherb/src/cairn-ehr && cargo test --workspace
```

This tree's full local gate takes roughly **two hours** over ~132 binaries — start it in the
background and do the documentation pass (Task 5) while it runs. Do **not** pipe it to `tail`.

If a run dies with exit 101 and **zero** `test result: FAILED` lines, that is a killed binary, not
a failure: the cause is macOS's one-time-per-binary Gatekeeper assessment (check `syspolicyd`'s CPU
with `top`). Re-exec the printed `target/debug/deps/<name>-<hash>` directly.

- [ ] **Step 10: Commit**

```bash
cd /Users/hherb/src/cairn-ehr
python3 scripts/check_closing_keywords.py
git add crates/cairn-node/src/actor_enrolment.rs crates/cairn-node/src/lib.rs \
        crates/cairn-node/src/main.rs crates/cairn-node/tests/device_actor_enrolment.rs \
        crates/cairn-node/tests/ cairn-gui/cairn-gui-live/
git commit -m "$(cat <<'EOF'
feat(#654): one enrolment rule -- nothing provisions on a write path

The CLI enrolled the node's key as a device actor on first use; `cairn-gui-live`
deliberately did not, because provisioning as a write-path side effect is the
shape ADR-0066 decision 6 forbids for the unwrap key (trap 2). So a node behaved
differently depending on which surface touched it first, and the reference
window's FIRST registration refused with a message naming a key, not a remedy.

FINDING: the issue understates its blast radius by fifteen. `ensure_registration
_actor` was not `patient-register`'s helper -- fifteen write subcommands called
it, and its own doc calls it "the headless-node/CLI convenience". Retiring it
from one would have left fourteen still provisioning silently.

So: `cairn-node init` enrols (an ordinary operator gains no new act -- M = 0),
`cairn-node enroll-device-actor` is the named remedy for a node that was not
initialised, most obviously one restored without its actor registry, and all
fifteen write paths now REQUIRE and refuse. The refusal is a DeliberateRefusal
(#651), so it is a verdict rather than an outage, and it names the command.

The kind-agnostic existence check moves across verbatim: `submit_event` resolves
a signer by `signing_key_id` alone, and a key mapping to two `actor_current`
rows nulls the actor_id of every event it ever authors (db/005).

NOT done here: `LiveData` is unchanged. Making its refusal actionable means a
sentence in the window's chrome, which is rendering, and rendering is slice 2c's.
`device_actor_enrolled` is public so 2c's `build_live_state` can probe at launch.

Co-Authored-By: Claude Opus 5 (1M context) <noreply@anthropic.com>
EOF
)"
```

---

## Task 5: the documentation pass

**Files:**
- Modify: `docs/HANDOVER.md` — the ⇒ NEXT block
- Modify: `docs/ROADMAP.md` — a dated entry
- Modify: `docs/superpowers/specs/2026-09-20-registration-search-funnel-ui-design.md` — the *Error
  handling* section's 2026-09-22 revision now describes a fixed thing
- Modify: `CONTRIBUTING.md` — only if it documents the CLI's first-use enrolment

Run this **while the two-hour root gate runs**, not after it.

- [ ] **Step 1: HANDOVER's ⇒ NEXT**

Rewrite it so 2c is the next slice with these four closed under it. Specifically:

- Delete *"⇒ FILED BY 2b's REVIEW PASS — #655–#660, and three of them should gate 2c"*'s entries
  for #651, #659 and #660, and the #654 paragraph. **Keep #655, #656, #657, #658** — untouched by
  this work.
- Add one short block naming the four durable rules this PR established:
  1. **`TokenStore::settle` is the only end of a `take` a caller should reach for.** `discard`
     still does not clear `in_flight`, and that is correct — clearing it is how two clicks produced
     two charts.
  2. **`--mock` can fail, one shot at a time**, and the failing `register` arm returns the
     attestation exactly as the live port does.
  3. **A verdict has two discriminators now**: `P0001` from the floor, `DeliberateRefusal` from
     Rust. Both live in or are read by `cairn_node::db_diagnosis`; **#655's `false` half is still
     wrong** and #652 should still gather all three P0001 homes.
  4. **Nothing provisions an actor on a write path.** `init` enrols, `enroll-device-actor` is the
     remedy, fifteen write commands refuse. **Never weaken `require_device_actor` to green a red
     fixture** — trap 2's instruction, one subsystem over.
- Add to 2c's owed list: **the launch-time `device_actor_enrolled` probe in `build_live_state`,
  and the chrome sentence for it** (#654's option 2, which this PR deliberately left to 2c).
- Keep every issue number that is still open. **Verify none is lost** — diff the set of `#NNN`
  references before and after:

```bash
cd /Users/hherb/src/cairn-ehr && git show HEAD:docs/HANDOVER.md | grep -o '#[0-9]\{2,\}' | sort -u > /tmp/before.txt
grep -o '#[0-9]\{2,\}' docs/HANDOVER.md | sort -u > /tmp/after.txt
diff /tmp/before.txt /tmp/after.txt
```

Every removal must be one of #651, #654, #659, #660. Anything else is an accident.

- [ ] **Step 2: ROADMAP entry**

Add a dated entry in the same voice as the `2026-09-22 — funnel UI slice 2b` entry above it:
what was decided, what it cost, and the fifteen-call-site finding. Update the existing
`#651`/`#654`/`#659`/`#660` mentions in that file's slice-2b entry to say **closed**, with the PR
number, rather than deleting them — this file is the project's memory of why.

- [ ] **Step 3: The design doc**

In *Error handling*, the third bullet of the 2026-09-22 (slice 2b) revision currently ends by
saying #651 pins the wrong behaviour. Add a dated note under it — **do not edit the 2026-09-22
text**, this project overlays rather than erasing:

```markdown
> **Resolved 2026-09-23 (slice 2c prerequisites).** A Rust-side pre-flight refusal is now a
> `cairn_node::db_diagnosis::DeliberateRefusal` and reaches the clerk as `Refused` (#651). The
> test that pinned the wrong behaviour expects the right one. What is still wrong is #655's half
> of the rule: a constraint violation, a privilege refusal and a "schema never loaded" are floor
> *decisions* carrying their own SQLSTATE, and they still land in `Unavailable`.
```

Add a second dated note under *Architecture → Commands* recording the #654 decision and that 2c
owes the launch-time probe.

- [ ] **Step 4: Prune both documents**

`docs/HANDOVER.md` is 1031 lines and `docs/ROADMAP.md` is 1008 against a 500-line guideline.
**Serving the reader beats hitting the number** — condense redundancy (the same rule restated in
three places), do not delete history that is load-bearing, and stop grinding once the redundancy is
gone. Re-run the issue-number diff from Step 1 after pruning.

- [ ] **Step 5: Verify the plan guard passes**

```bash
cd /Users/hherb/src/cairn-ehr && cargo test -p cairn-node --test paper_parity_plan_section
```

This plan's `## Paper-parity benchmark (§1.2)` section must contain **"Paper counterpart"**,
**"Steps"** and **"Time + cognitive load"** verbatim — it does; this step confirms it rather than
discovering it after a two-hour gate.

- [ ] **Step 6: Commit**

```bash
cd /Users/hherb/src/cairn-ehr
python3 scripts/check_closing_keywords.py
git add docs/ CONTRIBUTING.md
git commit -m "$(cat <<'EOF'
docs: the four traps under slice 2c are closed, and 2c is the window

HANDOVER's next block, a dated ROADMAP entry, and two dated overlay notes on the
funnel design page -- the project overlays rather than editing, so the
2026-09-22 text stands and the resolution sits under it.

Records what 2c still owes that this PR deliberately left it: the launch-time
`device_actor_enrolled` probe in `build_live_state` and the chrome sentence for
it, which is #654's option 2 and is rendering work.

Both documents pruned. Issue-number set diffed before and after; the only
removals are the four this PR closes.

Co-Authored-By: Claude Opus 5 (1M context) <noreply@anthropic.com>
EOF
)"
```

---

## Task 6: review, push, PR

- [ ] **Step 1: Self-review the whole branch**

```bash
cd /Users/hherb/src/cairn-ehr && git diff main...HEAD --stat && git diff main...HEAD
```

Check specifically:
- No `#[allow(...)]` was added to silence something.
- No test asserts merely that a call failed where it should assert *how*.
- Every new `pub` item has a doc comment that says why it exists, not what the next line does.
- No file crossed 500 lines that was not already over. `main.rs` **shrinks** in this PR; confirm it.
- `crypto_sink_names_are_genuine.rs` passes — the new test helper is named `lineage`, not `salt`.

- [ ] **Step 2: Run both trees' gates one final time**

```bash
cd /Users/hherb/src/cairn-ehr/cairn-gui && CAIRN_ALLOW_DB_SKIP=1 cargo test
cd /Users/hherb/src/cairn-ehr && cargo test --workspace
```

Plus the DB-gated suites, with a real connection string, in both trees. **Report what skipped.**

- [ ] **Step 3: Request a code review**

Use `superpowers:requesting-code-review` over the whole branch. Feed it this plan and the four
issues. Fix what it finds; file an issue for anything you cannot fix in place (`CLAUDE.md` rule 5).

- [ ] **Step 4: Push and open the PR**

```bash
cd /Users/hherb/src/cairn-ehr && git push -u origin feat/2c-prerequisites-654-651-659-660
```

The PR body must close all four issues with unambiguous keywords (`Closes #659`, `Closes #660`,
`Closes #651`, `Closes #654`), state the fifteen-call-site finding prominently, and say what was
deliberately **not** done (`LiveData` unchanged; only `dob_precision` converted; #655's half of the
rule still wrong). End it with:

```
🤖 Generated with [Claude Code](https://claude.com/claude-code)
```

**Open it even if something is unfinished** — as a draft, with the body saying what is done, what
is not, and what blocks it. `CLAUDE.md` rule 8: a branch that exists only on one machine is
invisible, and that cost a full session on 2026-09-07.
