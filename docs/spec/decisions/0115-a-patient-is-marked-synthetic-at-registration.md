# ADR-0115 — A patient is marked synthetic at registration

- **Status:** Accepted
- **Date:** 2026-11-28
- **Spec version at acceptance:** 0.117
- **Supersedes:** [ADR-0114](0114-a-node-says-at-genesis-that-its-data-is-invented.md), wholly. Nothing
  of ADR-0114 was built.
- **Refines:** [ADR-0061](0061-registration-is-an-act-that-carries-its-search.md) (one more fact a
  registration states)
- **Issues:** [#1219](https://github.com/cairn-ehr/cairn-ehr/issues/1219) (this decision; the build is
  owed), [#1207](https://github.com/cairn-ehr/cairn-ehr/issues/1207) and
  [#1220](https://github.com/cairn-ehr/cairn-ehr/issues/1220) (the node-level approach and the false
  premise that ended it), [#1214](https://github.com/cairn-ehr/cairn-ehr/issues/1214) (the synthetic
  cohort load)

## Context

ADR-0114 decided, the same day as this, that a node holding invented patients says so at genesis and
never shares a peering edge with a node for real records. It was withdrawn within hours, for two
reasons.

**Its premise was false.** It rested on "a peer is served only if this node has admitted it". That is
true of the node plane. It is not true of the clinical plane: `cairn-sync serve` ships events to any
puller and withholds only custody, deliberately ("withhold, never refuse the pull"), and a passing test
pins it (`an_unadmitted_puller_replicates_events_but_gains_no_custody`). The plan's first task, which
existed to test what the plan had only read, found this before any code was written (#1220).

**It solved the wrong problem.** The maintainer's ruling: synthetic data is usually used to test an
isolated node, and where it is not, it is meant for training. Training happens on the node where the
work happens, beside real patients, and a training chart has to reach the same terminals the real ones
do. A node that may never synchronise is the opposite of what training needs. It is enough to mark the
patient: everything recorded about a marked patient is then synthetic by derivation.

So the unit is the chart, which is what ADR-0114 had split off as #1219 and left for later.

The paper counterpart is a dummy chart with TRAINING stamped across its cover, kept in the same trolley
as the real ones because that is where staff learn to use the trolley.

## Decision

Decided by the maintainer, 2026-11-28.

1. **A registration may state that the patient is synthetic.** The statement is a field of the
   registration event (`identity.registration.asserted`), additive
   ([principle 11](../index.md#founding-principles-the-lens-for-every-decision)). A registration without
   it is of a real person, which is every chart that exists today.

2. **It is a fact of its own, not a registration class.** The three classes of
   [§5.3](../identity.md#53-registration-classes) say how much is known about who the patient is. Whether
   the patient exists is a different question, and every class needs it: an invented unidentified
   patient is what an emergency department trains on.

3. **Everything on the chart is synthetic by derivation.** No clinical event carries a mark. A
   medicine, a note or a result is synthetic because the chart it is filed under is. The wire core's
   event body is unchanged.

4. **Nothing is barred from synchronising.** A synthetic chart replicates like any other. There is no
   node-level statement, no refusal at pairing, and no separate backup format. ADR-0114's
   `init --synthetic` is withdrawn and not kept as a label: a label that enforces nothing invites trust
   it has not earned.

5. **A synthetic chart is never linked to a chart that is not.** The floor refuses an
   `identity.link.asserted` across the line, and the matcher never proposes one. This is a refusal, and
   not the usual hard veto that forces a human decision
   ([§5.2](../identity.md#52-matching-pipeline-safety-asymmetric-false-merge-worse-than-false-split)):
   no person should be able to say that an invented patient is a real one. It matters because invented
   identifiers are valid in form and can equal a real patient's, so the matcher will meet such pairs.
   Two synthetic charts may be linked, and training on the duplicate-repair workflow needs that.

6. **The mark is never changed.** No event makes a synthetic chart real or a real chart synthetic. A
   wrong mark is repaired the way any wrong-chart error is: a correct chart is registered and the
   events are reattributed to it ([§5.5](../identity.md#55-reattribution-one-primitive-tiered-workflows)).
   An overlay that flipped the mark would be simpler to use, and would make "synthetic" something a
   reader has to check the history of before believing.

7. **It is shown wherever the patient is shown.** A permanent band on the chart, and a mark on every
   search row, candidate list and worklist line that names the patient, so that the training John Smith
   cannot be picked for the real one unseen. The band is read, not answered: it is not a confirmation
   dialog. The registration is not sealed, so the mark is readable without a key.

8. **A count of patients says whether it includes them.** A duplicate sweep, a worklist, a results
   inbox or a report either leaves synthetic charts out or states that it has not. Which of those a
   given surface does is that surface's decision; saying nothing is not an option.

The canonical home is [identity §5.3](../identity.md#53-registration-classes).

## Not guaranteed

- **A node on a build that predates this shows a synthetic chart as a real one.** It does not know the
  field and ignores it. Nothing short of the bar this decision removes would prevent that. A site that
  trains with synthetic charts on a federation of mixed versions has unmarked invented patients on its
  older nodes, and must know it.
- **A wrong mark cannot yet be repaired.** Decision 6 names reattribution as the repair, and
  reattribution is specified ([§5.5](../identity.md#55-reattribution-one-primitive-tiered-workflows))
  and not built. Until it is, a real patient registered as synthetic by mistake stays so marked, with
  their record under the band, and the only remedy is to register them again and re-enter. The build of
  this decision should make the wrong mark hard to make, because for now it cannot be unmade.
- **It is a statement by whoever registers.** A chart registered without the mark is taken as real. An
  importer or a trainer who forgets it has made a real-looking patient, and nothing detects that.
- **Identifiers are not partitioned.** A synthetic chart can carry the IHI of a real patient. Decision 5
  stops them being linked; it does not stop a search by that identifier returning both, marked.
- **Charts registered before this is built are unmarked,** including every patient already loaded by
  `scripts/import_synthetic_cohort.py`. They are not converted: such a database is loaded again.

## Consequences

- **Easier:** training on the node where the work is done, with charts that reach every terminal. A
  periodic load of an invented cohort (#1214) into any node, without a node kept apart for it. The
  cohort loader registers each patient as synthetic and loses its guard against a node that holds
  anyone else. ReportTriage's rule that a synthetic record is never read beside a real feed gets a
  per-chart fact to check.
- **Harder:** every surface that lists or counts patients has to carry the mark or answer decision 8,
  and each new one from now on. The link floor and the matcher gain a rule. The mistake ADR-0114 was
  written against (invented patients arriving in a real record by an accidental sync) is now permitted
  by design, and what protects a clinician is the mark alone.
- **The bet:** that a mark which is always visible is enough, and that mixed-version federations which
  also train with synthetic charts are rare. We would know the bet has failed if a clinical act is
  recorded on a synthetic chart in the belief that the patient was real, or the reverse.
- **Mechanism, not policy** ([principle 9](../index.md#founding-principles-the-lens-for-every-decision)):
  Cairn ships the mark, the refusal to link, and the display. Whether a site trains on its live node,
  and with what, is the site's business.
- **On process:** ADR-0114 was accepted on a premise read from the code and not run. The plan written
  for it began with a task to test exactly such premises, and that task is what caught it. That order
  (decide, then test the premise) cost one withdrawn ADR; the cheaper order is to test the premise
  before the decision is written.
