# ADR-0068 — Provenance warns, never gates, on the restore path

- **Status:** Accepted (refines 0067)
- **Date:** 2026-09-10
- **Closes:** [#571](https://github.com/cairn-ehr/cairn-ehr/issues/571) — *decide and record: provenance
  does not gate the clinical plane on restore.*
- **Derives from:** [ADR-0067](0067-a-restore-reads-the-clinical-plane.md) (a restore reads the clinical
  plane), [ADR-0026](0026-node-durability-and-disaster-recovery.md) (node durability and disaster
  recovery), and **principle 3** (paper-parity — *confirmation dialogs are explicitly NOT an acceptable
  safety mechanism*).

---

## Context

DR slice 2a deferred one question explicitly to 2d: *"whether an unsigned segment should ever be
restorable without operator confirmation."*

2d's design answered it in §5.2, and the answer has two readings that the sentence supports equally:

> **Decision: clinical segments inherit the same `Provenance` treatment the node plane already gets.**
> An unsigned or non-sole-enroll-signed medium requires the operator's identity confirmation before its
> clinical records are applied — the third human act the paper-parity benchmark counts, and #512's
> `M = 3`.

The node plane's actual treatment is a **print**. Every `Provenance` match arm in the restore path of
`crates/cairn-node/src/main.rs` is a `println!` or an `eprintln!` — a stdout confirmation where the
marker is tamper-evident, a stderr warning where it is not; **nothing blocks, and nothing ever has.** So on
the first reading the clinical plane already inherits it, because the warnings print before any clinical
record is applied. On the second reading it is a blocking gate that exists nowhere in the tree.

What shipped is the first reading. **ADR-0067 recorded neither** — it does not mention provenance at
all. The record therefore carried a design sentence with two readings, a code path implementing one of
them, and no decision anywhere saying which. That is what #571 filed, and it is a decision rather than a
patch because an **unsigned CAIRNB3 medium restores every patient record on a stderr warning**, which is
either right and unwritten or wrong and unbuilt.

---

## Decision — Provenance warns; it never gates. On either plane.

`restore` prints what it was able to establish about the medium's self-marker and continues. There is no
confirmation prompt, no refusal, and no flag that turns one on. Four reasons, in the order they bind.

**1. Refusing converts a partial loss into a total one.** This is the ruling the slice has already made
everywhere else it came up, and there is no principled reason for provenance to be the exception: a
**torn tail** must not refuse a restore (2c reversed itself on exactly this point), an **`Unknown(tag)`
plane** must not refuse one, and a **legacy CAIRNB1/B2 medium** must not either. In each case the trade
is the same and `restore` is the one command where it is sharpest — an operator reaches it when
everything else has already failed, usually at the moment re-running the backup is impossible.

**2. A gate does not buy what it appears to buy.** Per-event signatures stop **forgery**, not
**omission**. An unsigned medium can have had records silently removed — a `revoke`, or an
`erasure.shred.asserted` — with no tamper evidence anywhere, and `chain_report` treats unsigned segments
as chain-verified on purpose, because an unavailable signing key must not make a medium unreadable. A
confirmation prompt detects none of that. It asks the operator to ratify an **identity**, not to attest a
**record set**. Blocking on it would trade a real recovery for a reassurance about a different question.

**3. Principle 3 forbids the mechanism by name.** *"Confirmation dialogs are explicitly NOT an acceptable
safety mechanism — they fail paper-parity; restore the physical affordance instead."* The physical
affordance is already present and is the strongest one in the system: the operator went to their own
off-site store and attached this medium. A dialog adds a keystroke, not a check.

**4. Input is not ceremony, and the difference is why the recovery-code prompt is not a counter-example.**
The restore path does hold one unavoidable interactive prompt — the old node's recovery code, which
unseals the local-state export. That prompt asks for a **secret only the operator holds and which nothing
can substitute for**; without it the custody key does not come back. A provenance confirmation asks the
operator to **ratify a judgment the machine has already made and printed**. The first is input. The
second is ceremony, and ceremony is what principle 3 rejects.

### What the operator is told instead

The five match arms are the whole of the treatment, grouped in four below because `Unsigned` and
`NoMarker` share one. They are deliberately unequal because the situations are:

- **`Signed`** — self-identity confirmed by a signed self-marker, tamper-evident. Printed to stdout, not
  a warning.
- **`Unsigned` on a CAIRNB3 medium whose marker came from a verified segment attestation** — also
  confirmed, and also stdout, because reporting it as unsigned would be a **false statement to a human at
  the exact moment they decide whether to trust a restore**. It carries the converged-peer splice note
  unconditionally.
- **`SignedFederated`** — a warning: the signature resolves self, but a converged peer's medium holds a
  byte-identical event set, so a peer's genuine marker could be spliced here and the signature alone
  cannot rule it out.
- **`Unsigned` / `NoMarker`** — a warning naming which of the two applies and why, and telling the
  operator to confirm the name and address printed below against **this** node.

Each of the two warnings ends by naming the check the operator can actually perform. That is the substitute for the
gate, and it is available at every rung including the ones where a gate would have been unavailable.

---

## Consequences

- **The third human act in DR slice 1's paper-parity section does not exist.** That section named the
  identity confirmation as the act making `M = 3`; there is no such act. This ADR does **not** therefore
  declare `M = 2` — [#512](https://github.com/cairn-ehr/cairn-ehr/issues/512)'s step count is re-derived
  by the measurement that issue owes, against the shipped command rather than against a plan, and the
  recovery-code prompt is a real act whose bundling that plan assumed rather than measured. **If the
  re-derivation falls outside the budget, that is the finding.**
- **2d's design test 18 is restated, not dropped.** *"Provenance gates the clinical plane"* becomes
  *"provenance warns before any clinical record is applied, and never blocks"* — the behaviour is
  testable either way, and an absent test nobody can find is what let the divergence sit.
- **The residual is now written where a reader looks for it.** #571's complaint was not that the
  behaviour was wrong; it was that the reasoning lived nowhere.

## What is still not true

- **An unsigned medium's omissions remain undetectable**, and this ADR does not change that. The remedy
  is a **signed capture**, not a prompt: a node that holds its signing key writes segment commitments
  that bind each record's `source_seq`, and the omission stops being silent. A medium captured without
  the key is a weaker artifact, and the honest surface for saying so is `verify-backup` — which
  [#567](https://github.com/cairn-ehr/cairn-ehr/issues/567) records as still federation-only, so it says
  nothing about the clinical plane a restore now applies. **#567 is where this gap belongs**, not a gate
  on the restore.
- **`segment_commitment` does not bind `attestation`/`attester_key`**
  ([#556](https://github.com/cairn-ehr/cairn-ehr/issues/556)), so a stripped human-authorship token is not
  caught by the commitment even on a signed medium. Free only until a release ships a CAIRNB3 writer.
- This ADR rules on **provenance only**. It says nothing about the several other ways a restore can
  recover less than the operator believes — a burned identity `seq`
  ([#549](https://github.com/cairn-ehr/cairn-ehr/issues/549)), an unopenable DEK
  ([#536](https://github.com/cairn-ehr/cairn-ehr/issues/536)), or a pen that cannot be drained.
