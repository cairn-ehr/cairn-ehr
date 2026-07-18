# Health Records Reinvented: What Happens When Clinicians Are in the Driver's Seat

*No vendor lock-in. Workflows that save time instead of stealing it. One system that scales from a
solo clinic to a national network. And AI assistants as first-class citizens — made safe to use.
An essay on what clinicians actually need from an electronic health record, and how Cairn is
designed to deliver it.*
{: .essay-lead }

---

## Thirty years, seventy-two systems

I have worked as a clinician in multiple countries for more than thirty years — from tertiary
hospitals in Germany, Norway, and Australia to remote Indigenous clinics and refugee camps in Papua
New Guinea. I have worked as a surgeon, in rural and remote general practice, and for the past
decade mainly in emergency medicine in remote hospitals.

Wherever I worked, there were "electronic health records". They came in many varieties and behind
many user interfaces, but all of them — in every country and every setting — had one thing in
common: they were unfit for clinical purpose. Not one of them supported the clinical workflow. The
best merely slowed us down compared to paper and were a nuisance; the majority forced us to record
untruthful data, made our work materially harder, and put our patients at risk.

I remember one number: **72**. That was the tally, the last time I counted, of the different record
systems I had used in my career. I stopped counting a few years ago. I will not name and shame any
particular product, because they should all hang their heads collectively: not a single one, in
thirty years, has deserved praise from an experienced clinician.

This is genuinely puzzling, because computerised records have undeniable advantages over paper.
Legibility, first and foremost — provided the interface doesn't make finding anything next to
impossible. Distribution — a record can be up to date in several places at once. And automated
assistance: knowledge support, dose checking, quality control that no paper chart can offer. The
raw material for something wonderful has been sitting there for decades. So why do clinicians
mostly hate the result?

One root cause stands out. Most of these systems are designed *for* — and often *by* —
administrators and managers: people far removed from the clinical front line, and generally as
clueless about clinical workflows as clinicians are about administrative requirements. Two worlds,
far apart, with only a small intersection. Some vendors keep a "token clinician" on board during
the (usually late) design phase; most pretend to take our advice and then quietly ignore it. The
result is faithful to its origins: a billing and audit instrument with a clinical veneer, handed to
the one profession that was never really in the room.

This essay is my answer to a simple question: what would the record look like if clinicians had
been in the driver's seat from the first line of code? The first half is the demand — what we want,
and what the ideal system would look like. The second half is the response — how
[Cairn](https://cairn-ehr.org), an open-source health record architecture I am building with
exactly this question as its founding brief, answers each demand not with a feature promise but
with a structural design decision.

## What clinicians want from an EHR

None of these are luxuries. Every one of them is something paper already gave us, or something a
computer could trivially give us if anyone had asked.

1. **It must be available all the time.** No downtime because the power failed or the network
   dropped. A record system that goes dark takes the whole department with it.
2. **It must not be slower to use than paper.** Every extra minute at the keyboard is a minute
   taken from a patient.
3. **It must not prevent us from doing anything we can do on paper** — with the sole exception of
   malfeasance, such as forging a record or silently backdating one.
4. **It must exchange data with the systems other clinicians use** — without extra work, without
   loss, without corruption.
5. **It should be better than paper where computers genuinely help:** dose and interaction
   checking, calculations, up-to-date guidelines — delivered inside our workflow, not in a separate
   module we have to go and visit.
6. **It must never force us to record a precise untruth.** No modal dialog may extort a diagnosis
   code, a date, or a dose that we cannot honestly vouch for as the price of saving our work.
7. **It must support our workflow rather than load extra cognitive work onto us.** We hold a
   patient's story in our heads while we work; every screen that makes us re-derive context is a
   screen that risks losing part of that story.
8. **It must never lock us out of the record while we are writing it.** Modal dialogs are the enemy
   of correct and concise documentation. I have used systems that would not let me view a lab
   report next to the progress note I was writing — systems where looking something up meant either
   losing the text I had already entered, or navigating a labyrinth of menus and finding my way
   back. On paper I simply lay the report beside my note. Software must do at least that well.
9. **It must be intuitive.** If a locum cannot use it safely on their first shift, it is not a
   clinical tool; it is a training burden.

And two more that paper never gave us, but that three decades of watching electronic systems fail
have added to the list:

10. **It must make identity errors survivable.** Records get attached to the wrong patient; two
    patients get merged who are not the same person. In most systems these mistakes are somewhere
    between painful and impossible to undo. They must be fully repairable — with an audit trail and
    with zero data loss.
11. **It must belong to no one but the people it serves.** Not to a vendor who can hold the data
    hostage at contract renewal, not to a cloud whose outage is our outage, not to a company whose
    acquisition ends the product.

## What the ideal EHR would look like

Take those demands seriously and a picture emerges.

**It runs and performs well on whatever hardware is available at the point of care.** In most
hospitals, clinicians queue and squabble for workstations, because workstations cost money — the
hardware and the per-seat licences — and they occupy space and power that clinical rooms don't
have. Software that runs on tiny, power-frugal computers, the kind a small UPS can carry through a
whole day's outage, changes that arithmetic completely.

**It keeps working when the network doesn't** — the way email does. Everything already on the local
node stays readable; new documentation keeps flowing; and when the connection returns, the nodes
synchronise. Reliably, automatically, with no human intervention and no "sync conflict" dialog
asking a nurse to adjudicate a distributed-systems problem at 3 a.m.

**It lets clinicians document in their own language, in their own words, in their own style.** It
never forces an imprecise truth into false precision, and never forces a falsehood at all. I have
worked with systems that made us pick a diagnosis from a fixed list before the record would save —
and when the correct diagnosis, or anything remotely near it, was not on the list, we picked a
wrong one, because the alternative was losing the note. Those consistently wrong diagnoses then
"informed" public-health statistics and funding decisions. Garbage in, garbage out, and at the end
of that chain: poorer care at higher cost, built on data everyone involved knew was false the
moment it was entered.

**It facilitates the workflow instead of interrupting it.** Clinical dashboards that show exactly
the information the current decision needs, at the moment it is needed. Dosing insulin? Show me the
trajectory of blood sugars against the insulin already given, most recent first, alongside the
handful of parameters that actually bear on the decision — the HbA1c as the marker of long-term
control, existing complications, the co-medication that alters glucose or insulin response. That
is not artificial intelligence; it is elementary respect for how clinical decisions are made.

**It shares the record with everyone else caring for the patient — seamlessly and without
ceremony.** When my patient leaves hospital, their family doctor can already read my actual notes,
rather than waiting weeks for a discharge summary written retrospectively by an intern who never
met the patient. And when a patient arrives in an emergency department anywhere, the treating
physician sees the history immediately — not after a records clerk wakes up in the morning.

**It keeps the patient's trust while keeping the patient safe.** Confidentiality and clinical
safety pull in opposite directions — the sensitive episode a patient wants sealed may be exactly
the fact a future clinician needs to be warned about. The ideal record refuses to sacrifice either.

**And it lets machine assistance in — on our terms.** AI that drafts, summarises, cross-checks, and
flags is coming to clinical work whether the record systems are ready or not. The ideal record
neither bans it nor blindly trusts it: machine contributions are welcome, permanently labelled as
what they are, and structurally incapable of corrupting the record — with accountability that
always, traceably, rests with an identifiable human who accepted responsibility.

That is the demand side. Now the response.

## How Cairn answers — design decisions, not feature promises

Cairn is an offline-first, vendor-independent electronic health record, developed in the open under
the AGPL-3.0 licence. Its architecture specification is complete — every open design question has
been resolved and logged in an immutable decision record — and its hardest technical bets have been
validated in working code before any product was built on top of them. The first clinical slices
are under construction now.

What makes Cairn different is not a feature list. Features can be promised by anyone and removed by
the next product manager. Cairn answers each clinical demand with a *structural* decision — a
property of the architecture that cannot be quietly walked back later. Here is the mapping.

### Always available, because every node is the whole system

When a network partitions, a system must choose: stay consistent, or stay available. Cairn chooses
availability, unconditionally. A clinician must always be able to read the locally relevant record
and write new documentation, network or no network — because paper never returns "connection lost",
and no system that does can claim to replace it.

That choice has consequences, and Cairn embraces them. There is no central server that owns the
truth. Every node — a workstation, a clinic, a hospital, a regional hub — carries a full, working
copy of the system and of the records relevant to it. The topology is *fractal*: one codebase at
every tier, from a single machine in a bush clinic to a national network; a node's role is
configuration, not a different product. Synchronisation between any two nodes is mathematically
safe by construction (more on that below), so reconnection after an outage is a non-event, not a
recovery procedure.

And because the whole system runs in one codebase on commodity hardware, "whatever is available at
the point of care" is taken literally: a complete Cairn node has been demonstrated running on a
Raspberry Pi — a computer the size of a pack of cards that a small UPS can power for a day — and
its database layer has been run on an ordinary Android phone. Synchronisation has been validated
over a real satellite link with nearly a second of round-trip delay — and not a laboratory
simulation of one. The two nodes were my laptop in hospital accommodation in Bamaga, at the
northern tip of Cape York, and a machine at my home some 2,400 kilometres south on the New South
Wales coast: WireGuard over Starlink at both ends — a residential dish at the southern end, and at
Bamaga a Starlink Mini lying unfastened on the roof while a storm blew through. If sync shrugs that off, a hospital basement
holds no terrors. These are not projections; they are passed tests.

### Never slower than paper — paper-parity as law

Cairn elevates the speed demand from an aspiration to a governing law: **no clinical workflow may
be slower, harder, more cognitively demanding, or less capable than its paper equivalent.** The
only excluded paper "capabilities" are the fraudulent ones — silent falsification, untraceable
backdating. Every new workflow must name its paper counterpart and be benchmarked against it in
time, steps, and cognitive load.

One consequence deserves special mention, because it breaks with two decades of lazy convention:
**confirmation dialogs are explicitly rejected as a safety mechanism.** A clinician who is
interrupted a hundred times a day learns to click "OK" reflexively; the dialog provides legal
cover, not safety, and it fails the paper benchmark every single time. Where software has a safety
problem — documenting on the wrong chart, say — Cairn's rule is to restore the *physical
affordance* that made the error hard on paper (you were holding one specific folder), not to
interrogate the user.

### Never a forced untruth — uncertainty as a first-class value

Cairn's data model is built on a principle it states baldly: **an imprecise near-truth always beats
a precise untruth.** Uncertainty, imprecision, ranges, and an explicit *unknown* — distinct from
*not yet asked*, and distinct again from *the patient declined to say* — are first-class recordable
values everywhere. No required field, anywhere in the system, may be satisfiable only by
fabrication. The mandatory diagnosis picker with no honest option is not a design flaw in Cairn; it
is a violation of the architecture.

Time — the thing most systems most cheerfully falsify — gets the full treatment. Every event
carries two timestamps: the objective moment it was recorded, and the clinically asserted time it
refers to ("the chest pain started around midnight"). The asserted time is freely backdatable,
because honest documentation demands it; the recorded time is tamper-evident, because honest
auditing demands that too. Where the two tell conflicting stories, the conflict is flagged for a
human — never silently "resolved" by the machine.

Certainty, when it improves, is added later by overlay. It is never extorted up front.

### Nothing is ever lost — and everything stays readable

Every clinical entry in Cairn is an immutable, cryptographically signed event. Corrections do not
overwrite — they are new events that reference what they correct, so the record always shows both
what is believed now and what was believed at the time, and by whom. This is exactly the paper
discipline of the single ruled-through line with initials, made rigorous: never erase, always
overlay.

This is also, quietly, the mechanism that makes synchronisation trustworthy. Because events are
immutable, merging two nodes' histories is a *set union* — each side simply acquires the events it
was missing. Set union cannot conflict, cannot lose data, and gives the same answer no matter how
many times, in what order, or over how flaky a link it runs. The most dangerous operation in
distributed health records — the merge — is engineered out of existence.

And because a record is only as good as its readability decades from now, every event carries a
plain-text rendering of itself, and the data schema is only ever extended, never broken: a record
written today must still be legible, as written, to whatever software — or whatever human — reads
it in fifty years.

### Identity errors are survivable — never merge, always link

In Cairn, patient identity is treated as what it clinically is: **a claim, never a fact.** Records
are never merged into one another; they are *linked* by explicit, signed, auditable identity events
— and every one of those events can be countermanded by a later one. Linked in error? Unlink, with
the reason on the record. A document attached to the wrong patient? Reattribute it, auditable,
reversible, with nothing lost. The identity mistakes that in conventional systems demand a vendor
ticket and a prayer are, in Cairn, ordinary auditable events that any authorised clinician can
apply and any later clinician can inspect.

### Exchange without friction — one record, many front-ends

Cairn inverts the interoperability problem. Conventional systems are walled gardens that grudgingly
exchange summaries through interface engines; every fence between them is a place where data is
lost, mangled, or delayed. In Cairn, what makes any two nodes interoperable is the signed event
core itself — the wire format, the sync rules, the identity algebra — and *nothing above it*. No
API, no policy layer, no user interface sits on the path between nodes.

The safety and compatibility floor is enforced in the database itself, unbypassably: even software
talking raw SQL to a node cannot produce an invalid or wire-incompatible event. Above that floor,
user interfaces may proliferate freely — an emergency department UI, a general-practice UI, a
bespoke UI for a single specialised clinic — many front-ends, one record. A badly designed
front-end can produce a note its clinic finds unhelpful; it *cannot* produce a record another node
can't read.

This is what dissolves the discharge-summary farce. When hospital and family doctor both run Cairn
nodes, the family doctor's node simply syncs the actual notes as they are written — the "summary"
becomes a courtesy narrative, not the sole carrier of truth. And for the world outside the mesh,
Cairn speaks standard FHIR through a dedicated façade — interoperability with the systems that
exist, without letting their limitations dictate the internal model.

### Decision support that fits the workflow — and can't corrupt the record

That insulin dashboard — trajectory of glucose against doses given, the HbA1c, the complications,
the interfering co-medication, exactly when the dosing decision is being made — is precisely the
kind of thing Cairn's layering is built to make cheap. Dashboards, checkers, and calculators live
*above* the enforcement floor: they read the same event stream everyone else does and they write
through the same validated gate everyone else must use. A hospital, a research group, or a single
motivated registrar can build the decision-support view their unit needs, without asking a vendor's
permission and without any possibility of breaking the record underneath. The record is
infrastructure; the workflow tooling on top is a garden anyone may plant in.

### Confidential *and* safe — not one at the other's expense

Cairn refuses the usual trade. A patient may seal a sensitive episode — the content becomes
cryptographically inaccessible without the appropriate key custody. But a sealed episode that
carries future clinical risk still emits a de-identified, severity-graded *safety projection*: the
warning without the story. The antenatal clinician is told there is a sensitisation risk that
matters for this pregnancy — not the confidential history behind it. Emergency access
("break-glass") is not a hole in the wall but an audited use of a key: possible when it must be,
and permanently on the record that it happened.

Even the right to erasure is honoured within the append-only design: erasure is implemented as the
destruction of encryption keys — provable, auditable, and honest about exactly what it does and
does not guarantee.

### AI assistants as first-class citizens — made safe to use

Most record systems will bolt AI on the way they bolted everything else on: as an afterthought,
with the accountability questions waved away. Cairn was designed for it from first principles,
starting with an honest one: authorship and accountability are not the same thing. Every
contributor to a note — human or machine — is recorded in the event itself, permanently. But
*responsibility* is a separate, explicit act: a signature proves who produced content; an
attestation records who vouches for it. An AI draft is labelled as an AI draft forever, and the
human who reviewed and accepted it is on the record as having done so. No laundering of machine
output into apparent human prose; no ambiguity, years later, about who stood behind a decision.

The machine itself is held to a standard no human clinician could be: an AI assistant in Cairn is a
*registered actor* whose identity is pinned to the exact model and configuration it runs — change
the model, and it is a different actor. If a model version is later found to be flawed, every
contribution it ever made is identifiable and can be flagged for review in one sweep — a recall
mechanism, exactly as medicine already does for a faulty batch of a drug.

And the floor holds. Because validation lives unbypassably in the database, an AI agent — however
capable, however misbehaving — physically cannot forge another author's signature, alter history,
or write around the record's rules. This is not a policy statement; it was tested by
red-teaming: a deliberately hostile agent, granted direct database access, could not break the
contract. That is what "first-class citizen, made safe" means: not a chat window bolted to the
side, but a colleague with a name tag, a scope of practice, and no ability to falsify the chart.

### No lock-in — by construction, not by promise

Every mechanism above serves a mission that is explicitly anti-capture, and the project's own
governance is built to the same standard. The entire codebase is AGPL-3.0 — every improvement,
including improvements made by commercial operators, must remain open. The specification and every
design decision are public. The data lives in PostgreSQL on hardware you own; there is no mandatory
cloud, no licence server, no per-seat meter. The full architecture documentation means a competent
team could re-implement the system from the documents alone — which is precisely the point: the
exit door is a load-bearing part of the building. A vendor cannot hold hostage what, by
construction, no vendor controls.

## Where this stands, and an invitation

I want to be precise about status, because health IT has heard enough vapourware promises. Cairn
today is a completed, published architecture; a set of passed proof-of-concept trials for its
riskiest bets — synchronisation over a genuine high-latency satellite link, a full node on a
Raspberry Pi, the database layer on an Android phone, the write-contract floor held against a
hostile AI agent with direct database access; and the first production clinical components now
being built on that proven spine, slice by slice, in the open. It is not yet a product you can
deploy in your hospital. It is the foundation for one, built in the right order: the hard,
unforgiving, safety-critical parts first, validated before anything was stacked on top of them.

The design method that got it this far is one I intend to keep: take a real clinical failure —
mine, or one a colleague brings — and test it against the architecture until the architecture
either absorbs it or bends. Seventy-two systems taught me where the bodies are buried. If you have
worked the front line, you know where more of them are.

So this is the invitation. If you are a clinician: bring your worst war stories — the workflow that
made you slower, the dialog that made you lie, the merge that could not be undone. They are design
input of the highest grade, and they are exactly what this project runs on. If you are an engineer:
the specification, the decision log, and the code are public, and the problems are as hard and as
worthwhile as any in software. And if you are an administrator — you are genuinely welcome too. The
record that serves clinicians honestly turns out to serve you better as well: data that was never
falsified at entry is the only data worth funding decisions downstream.

For thirty years I have used systems built by people who never had to use them at three in the
morning. Cairn is what happens when the people who do take the wheel.

*Cairn is developed in the open at
[github.com/cairn-ehr/cairn-ehr](https://github.com/cairn-ehr/cairn-ehr), AGPL-3.0. The
[specification](../spec/index.md), the [decision log](../spec/decisions/README.md), and the
companion essays on this site go as deep as you care to follow.*
