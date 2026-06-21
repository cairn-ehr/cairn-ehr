# The Chart That Stays Up: Designing a Fractal-Topology EHR from the Clinician's Chair

*An essay on what it takes to build an electronic health record like Cairn — the architecture,
the reasoning behind it, and why the people who have been failed by these systems are the right
people to design the next one.*
{: .essay-lead }

---

## Starting from the wrong end

Most electronic health records are designed from the billing office outward. The data model is
shaped by what a payer needs to reimburse a claim, the workflow by what an administrator needs to
audit, and the clinician — the one person in the building who actually touches the patient — is
handed whatever is left over. The result is familiar to anyone who has worked a shift: a screen
that demands a date of birth before it will let you treat a hypotensive trauma patient who arrived
with no name; a mandatory drop-down with no honest answer; a login that times out between the
question and the chart; a network hiccup that takes the whole department dark. These are not bugs.
They are the faithful expression of whose interests the system was built to serve.

Cairn begins by inverting that. There is no vendor in the room, no revenue that depends on trapping
data, no proprietary layer anyone must license. That absence is not a marketing posture; it is a
design constraint with teeth. Because nothing is incentivised to keep the hard problems hard, a
single question is allowed to drive every decision: **what actually happens at the point of care,
including at three in the morning when the network is down.** Everything below follows from taking
that question literally.

The thesis of this essay is simple. An EHR that earns a clinician's trust must be three things at
once — *available*, *honest*, and *fast* — and each of those words turns out to be an architectural
commitment, not a feature you can bolt on later. Availability forces a particular topology and a
particular theory of synchronisation. Honesty forces a particular data model, one that can hold the
uncertainty real medicine is made of instead of papering over it. Speed forces a discipline about
the clinician's time that most systems abandon the moment a confirmation dialog seems easier than
thinking. Get any one of the three wrong and the clinicians quietly route around you — onto sticky
notes, onto the paper chart, onto the verbal handover — and the record stops being the record.

---

## Why "fractal" is the load-bearing word

Health care is not one shape. It is a one-room rural practice on a solar panel and an intermittent
mobile signal, and it is a thousand-bed tertiary hospital with a server room, and it is everything
in between, and it is the ambulance and the disaster tent and the ship. The reflex of the industry
has been to build different products for these — a "lite" edition for the clinic, an enterprise
edition for the hospital, a cloud tier for whoever can afford the subscription — and then to make
interoperability between them a paid feature, or a standards committee's problem, or nobody's.

Cairn takes the opposite vow: **one codebase at every tier.** A node's role — workstation,
department, facility, region, nation — is *configuration*, not a different program. The topology is
fractal because the same structure repeats at every scale: each node holds a full database, each
node is write-capable, each node can stand alone if it must, and each node syncs upward to a parent
when it can. Zoom in on a national hub and you see the same machine as the clinic Pi, just holding a
larger scope and more peers. This is the literal meaning of fractal: self-similar across scales.

Why insist on this? Three reasons, all of them mission, not convenience.

First, **anti-capture.** The instant you ship two codebases, the expensive one becomes the place
the hard features live, and the cheap one becomes the funnel that pressures you to upgrade. A single
codebase scaled by configuration removes the commercial gradient that turns "interoperability" into
a lever. There is no edition to be locked out of.

Second, **reachability.** The same software has to run on Raspberry-Pi-class hardware with flaky
power and 2G-grade connectivity *and* on a hospital cluster. If those are different products, the
low-resource clinic gets the neglected one — which is exactly the population most EHR efforts
abandon. One codebase means the clinic and the tertiary centre are debugging, hardening, and
improving the *same* code. The rural deployment is a first-class citizen because it is, byte for
byte, the same citizen.

Third, **a clean theory of "what is a node."** Because every computing node is a full PostgreSQL
instance, the cleverness of the system — the merge logic, the identity algebra, the projections —
can live *inside the database* and therefore run everywhere, Pi included. This is the "fat Postgres,
thin daemon" decision: push the safety-critical intelligence into the one substrate that is present
at every tier, and keep the network-facing daemon as small and dumb as possible. Tablets, carts and
phones are explicitly *not* autonomous nodes; they are thin clients that attach to a nearby full
node, because a device that cannot survive a partition by itself must never be mistaken for one that
can. The boundary between "can stand alone" and "cannot" is drawn deliberately and never blurred.

The fractal commitment is what makes the rest of the architecture *possible to state once*. You do
not design availability for the clinic and again for the hospital; you design it once, for a node,
and the topology inherits it at every scale.

---

## Availability: the chart must outlive the network

Ask a clinician what the worst moment with their current EHR was and a startling number will
describe the same scene: the system went down, and for an hour or a day the safest thing in the
building was the printout someone had the foresight to make. An EHR that can be taken away by a
backhoe through a fibre line, or a regional outage, or a lightning strike on the uplink, has
mistaken a *convenience* for *infrastructure*. Paper never had downtime. That is the bar.

So Cairn makes an unusual and consequential choice at the level of distributed-systems theory: in
the language of the CAP theorem, it chooses **AP — availability and partition-tolerance — over
strict consistency.** During a network partition, a clinician must *always* be able to read the
locally relevant record and write new clinical data. Synchronisation catches up later. We do not
merely tolerate eventual consistency; we design the data model so that eventual consistency is
*clinically safe* rather than a euphemism for "your colleague's note and yours just overwrote each
other."

This is where availability stops being a slogan and becomes a constraint on the data model. If two
nodes can both accept writes during a partition — and they must, or someone cannot chart — then when
they reconnect, the two divergent histories have to be reconciled. The dangerous way to do that is a
*merge*: take two versions of a row and compute a winner. Last-writer-wins on a clinical field is
silent data loss, and silent data loss in a medical record is how people get hurt. Cairn refuses the
whole category of problem by making the clinical record **append-only**. Nothing is ever updated in
place. Every clinical fact is an immutable, signed event. A correction is not an edit; it is a *new*
event that references the original — which is, not coincidentally, exactly how medico-legal
documentation has always worked on paper. You strike through, you initial, you write the corrected
line below; the original stays legible underneath.

Once the record is append-only, reconciliation stops being a merge and becomes a **set union.** Two
nodes that diverged simply exchange the events the other is missing and insert them. There is no
winner to compute, because nothing was overwritten. Insertion is idempotent — every event carries a
globally unique, time-ordered identifier, so applying the same event twice is a no-op — which means
sync is safe to retry, safe to interrupt, safe to resume, and safe to run over a USB stick carried
between two sites that will never see the same network. The append-only log is what turns the
terrifying problem (distributed write reconciliation in a life-critical system) into a boring one (a
deduplicating insert). That is the single most important move in the architecture, and everything
clinical-safety-related leans on it.

Causal order is preserved by Hybrid Logical Clocks, so that even across nodes whose wall clocks
disagree, the system can reconstruct *what was known when*. And the topology degrades gracefully,
rung by rung: internet down, the facility runs on its own server; intranet down, the department
server is the local master; department server down, the workstation runs standalone on its mirrored
scope. At the bottom of the ladder is a single Pi-class box that can read its charts and accept new
ones with nothing else in the world reachable. The grid goes down; the chart stays up.

There is one more piece of availability that is easy to miss and clinically vital: **honest assembly
state.** Because a chart is now an assembly of parts that arrive from different nodes at different
times, it may be *incomplete* — and the system must say so, as a first-class clinical fact. The
chart shows when it last synced ("last synced with parent 4 h ago"), and where it can, it shows
*known-missing* parts: the parent advertised five episodes and only three arrived; a sibling node is
reachable but not yet synced. On paper, the other ward's notes were simply, invisibly absent — you
did not know what you could not see. Making absence *visible* is a genuine safety gain with no paper
equivalent, and it is the honest counterpart to availability: the system would rather tell you "this
may be partial" than let you mistake an island for the whole world.

---

## Honesty: the war on precise falsehood

If availability is the most important *structural* commitment, the war on precise falsehood is the
most important *clinical* one, and it is the place where a clinician-led design diverges most
violently from an administrator-led one.

Here is the failure, in its purest form. An unconscious patient arrives. You do not know their date
of birth. The registration screen will not advance without one. So you — or the clerk, under time
pressure, a hundred times a year — type `01/01/1900`, or today's date, or a plausible guess, and the
system swallows it as fact. That fabricated precision now propagates everywhere: into the identity
matcher that will try to link this visit to the patient's real prior records, into the age-based
dosing logic, into the population statistics, into the next clinician's mental model. **An honest
"unknown" would have been weighted correctly by every one of those consumers. A confident falsehood
actively misleads them.** The system did not capture truth; it manufactured a lie and then trusted
it.

Cairn's fourth founding principle is the direct answer: *an imprecise near-truth always beats a
precise untruth.* And it is not a sentiment — it is enforced in the type system of the data model.

Uncertainty is made **first-class and recordable.** A date can be known to the year, the month, the
day, or marked "circa." A value can be an interval — "50–60 years old," "two to three days,"
"sometime overnight." Crucially, the model distinguishes three things that almost every EHR collapses
into a single empty cell: `null` (nobody has asked), `unknown` (asked, but genuinely not
established), and `refused` (the patient declined to say). These are *clinically distinct facts*. "We
never asked about alcohol use," "we asked and the patient doesn't know," and "we asked and the
patient refused to answer" mean completely different things at the bedside, and a record that cannot
tell them apart is lying by omission.

From this follows a normative rule with real bite: **no required field may be satisfiable only by
fabrication.** If a workflow genuinely needs a field, that field must accept an honest uncertainty
value. You may compel an answer; you may not compel a *fake* one. This single rule would, if
enforced, eliminate an entire genus of clinical data corruption.

And because the record is append-only, certainty is allowed to **refine over time by overlay** rather
than being forced up front. "Circa 2019" today; "12 March 2019, confirmed from old records" as a
later overlaying event when the notes arrive. The estimate is never erased; it is layered over. The
record grows *more* precise as the world reveals itself, which is exactly how clinical knowledge
actually accumulates — provisionally, then confirmed — instead of demanding false precision at the
one moment you have the least information.

The same honesty governs *time itself*, which in medicine is rarely simple. Cairn keeps two
timestamps on every event. `t_recorded` is objective: when the system actually received the event,
anchored to the Hybrid Logical Clock, and it is a *ceiling* you cannot back-date past. `t_effective`
is the clinical claim: when the thing the note describes actually happened — and it is freely
back-datable, because a note written at 18:00 about an event at 14:30 is normal, honest, daily
practice. The chart can show you either lens: "as it happened" or "as it was recorded," which is
itself an audit affordance paper never had. Disagreement between the two is the *expected* case, never
an error. Only a genuine *impossibility* — a treatment whose effective time precedes the patient's
recorded arrival at the facility — is flagged as a clash. And on a clash, the system does the
disciplined thing: **it surfaces the contradiction and stops.** It never silently reorders, never
picks a winner, never erases. Either timestamp could be the wrong one, and only the humans who were
there can reconcile it — by adding a new overlaying event with a full audit trail. Forcing the
machine to choose would manufacture exactly the precise untruth the whole principle forbids.

Notice the through-line. Append-only, acknowledged uncertainty, and the two-timestamp model are not
three features; they are one stance — *the record's job is to be honest about what is and isn't
known, and to never destroy the path by which it became known* — expressed three times. A clinician
recognises this stance instantly, because it is how a good paper chart already behaves. The novelty
is enforcing it in a database that a thousand nodes can write to at once.

The same honesty even governs *deletion*. In an append-only, multi-replica, sometimes-archived-to-
write-once-media world, you cannot promise that a fact has been scrubbed from every copy in
existence — an offline node, an old backup, a detached drive may still hold it. So Cairn does not
pretend. Erasure is implemented as the destruction of an encryption key (crypto-shredding) rather
than the deletion of a row, and it is *declared, never guaranteed.* The strongest honest claim the
system will make is: *"to our knowledge, we have erased all copies in our existence."* That is the
truth, so that is what it says. An EHR that promised more would be lying, and this one has made a
constitution out of not lying.

---

## Speed: the clinician's time is the scarcest resource in the building

The third leg is the one most often sacrificed, because its costs are diffuse and its victims do not
file bug reports — they just get slower, and tireder, and a little less safe, all day, forever. Cairn
elevates it to **governing law**: the *paper-parity principle.* No clinical workflow may be slower,
more difficult, more cognitively demanding, or impossible compared to its paper-record equivalent.
Every workflow must be able to name its paper-era counterpart and be benchmarked against it in
**time, steps, and cognitive load.** A workflow that loses to paper is not a preference miss; it is a
*defect*, tracked as one.

This sounds modest until you see how sharp the knife is.

The benchmark is the **lived** workflow, not the demo. It counts the shared-workstation login, the
latency under real load, the interruption when you are pulled to a crashing patient and the
resumption ten minutes later. Paper's baseline — grab the chart from the rack, write — had no login
and no spinner. So when a digital workflow loses on parity because of round-trip latency to a server,
that is declared an *architecture* defect, not a UI polish item. And the structural answer is already
in hand: local-first reads and writes against the node's own database. **Paper-parity and offline-
first turn out to be the same requirement seen from two angles** — both say "the work happens here,
now, against something physically in front of you," and both forbid making the clinician wait on a
network. "Never make the user wait if engineering can avoid it" is the latency limb of the law:
default to the most likely choice, do the heavy work in the background while the clinician already
proceeds, and cache-and-hide rather than cache-and-clear so re-display is instant. Paper had no
spinner; neither may we.

The sharpest blade, though, is aimed at the industry's favourite safety theatre: **the confirmation
dialog is explicitly *not* an acceptable safety mechanism.** "Are you sure? OK / Cancel" is the
universal EHR reflex, and it is worse than useless — it trains click-through, so by the hundredth
time it is reflexive muscle memory that protects no one and costs everyone a half-second and a
flicker of attention. Cairn forbids it as a safety device. When a digital workflow breeds an error
that paper didn't — the classic case is charting on the *wrong patient's* record, a misfile that the
physical chart made almost impossible — the design heuristic is to ask *which physical affordance of
paper suppressed that error* and restore its semantics, before reaching for an alert. Paper suppressed
wrong-chart errors through *possession*: one chart, in one hand, open on the desk. The digital
restoration is possession semantics at the point of care, not a pop-up asking you to confirm the
patient's name you are no longer really reading.

There is exactly one friction the law permits, and its rarity is the whole point. A genuinely
*irreversible* act — a cryptographic erasure, a repudiation — earns a **forced-rationale gate**: not
a checkbox, but a demand for a substantive, recorded reason that cannot be click-throughed. Because
append-only plus overlay makes almost everything reversible (an ordinary "delete" merely suppresses a
rendering; the event is still there), the set of acts that qualify collapses to a handful per year.
That scarcity is what keeps the friction meaningful: when the system finally does stop and ask you to
explain yourself, you know it means it. *Never block the reversible; for the irreversible few, don't
confirm — demand a reason and record it.*

The same parity logic re-shapes notifications, where modern EHRs have done perhaps their greatest
damage. Paper was almost entirely *pull*: you saw a result when you picked up the chart. It had a few
high-value *pushes* — the critical-value phone callback with read-back and escalation, the allergy
sticker on the cover — and that was the lot. Deployed EHRs inverted this into everything-push and
manufactured alert fatigue, then dressed the regression as a feature. **More notifications is one of
the few digital "gains" that is not automatically better.** Parity prescribes inverting it back to
mostly-pull with a few precious pushes, and it names the un-removable floor: the critical-value
callback stays. Demoting the priority of the flood of normal results is allowed, because the human can
still see them; *hiding* them or auto-resolving them is not, because that forecloses a decision the
clinician would otherwise make. The line is drawn precisely where attention is either preserved or
stolen.

---

## How honesty, availability and speed are actually held together

It would be easy to read the three commitments as in tension — honesty wants to record everything,
speed wants to demand nothing, availability wants every node writing at once — and in most systems
they would be. What makes Cairn coherent rather than a pile of compromises is that a small number of
structural decisions serve all three at once.

The **append-only event log** is the keystone. It makes availability safe (sync is set union, not
merge), it makes honesty structural (corrections overlay, nothing is destroyed, the path to knowledge
is preserved), and it makes speed achievable (writes are local inserts against your own database,
never a negotiation with a server). One decision, three payoffs.

**Identity as a claim, never a fact** is the same move applied to *who the patient is.* Patients
arrive unidentified, misidentified, and sometimes deliberately under a false name; prevention can
never be complete. So instead of betting everything on getting identity right at the front door —
and then making it agony to fix when it's wrong — Cairn treats identity as a stream of append-only,
auditable events over immortal patient identifiers: *never merge, always link; never erase, always
overlay.* Two records that turn out to be one person are *linked*, not fused, so the link can be cut
again without data loss if it was wrong. A record attributed to the wrong patient is *reattributed*
by a new event, leaving the trail intact. Every identity error — accidental or fraudulent — is
repairable by an event, fast and forensically clean, because *repair* was designed to be a first-
class operation rather than a database surgery. This is acknowledged uncertainty (principle 4) and
append-only honesty (principle 1) reaching into the most error-prone corner of clinical data and
refusing to let a wrong guess become permanent damage.

And crucially, identity repair leans directly on the honesty principle: a matcher that is fed honest
"unknowns" instead of fabricated dates of birth makes *better* link decisions, and is engineered to
fail safe — a false *split* (two records for one person, later linkable) is recoverable, while a false
*merge* (two people fused into one chart) is the dangerous one, so the matcher is biased to withhold
rather than to guess. The war on precise falsehood is not a separate idealism; it is what keeps the
identity layer safe.

Underneath all of it sits a deliberate engineering discipline about *where* the dangerous code lives.
Components whose defects could silently corrupt the record, mis-merge two patients, or leak data are
pushed into places where whole classes of error become *unrepresentable* — memory-safe, strictly-
typed Rust, or enforced directly by the database as constraints and validated write paths — and they
are optimised above all for **reviewer-legibility**, because in a system this consequential the
binding constraint is no longer how fast you can write code but how confidently a human can verify it
is correct. The part of the system that most needs rigorous review is kept the *smallest*. By
contrast, the advisory, caught-immediately, cosmetic parts — the probabilistic matcher, the FHIR
interoperability façade, the UI backends — are free to optimise for iteration speed in whatever
language fits. The rule is not "use the safe language everywhere"; it is **choose each component's
substrate by the blast radius of its defects.** And the integration substrate that lets all these
pieces talk without brittle coupling is the one thing present at every node, every tier: the
PostgreSQL database itself.

This is also what lets a thousand different front-ends coexist without fragmenting the record. The
contract that makes any node interoperable with any other is the signed, append-only *event core* —
and nothing above it sits on the path between nodes. The compatibility floor is enforced unbypassably
*in the database*, so even a bespoke client talking raw SQL cannot emit a wire-incompatible event. A
clinic can build whatever UI suits its workflow; it can get its own local policy wrong; but it can
*never* produce an event that another node cannot read and trust. Many front-ends, one record. That
is how UI pluralism — a genuine clinical good, because the ED and the ICU and the rural GP do not want
the same screen — is reconciled with the anti-capture mission of a single interoperable substrate.

---

## Why it has to be clinicians in the driver's seat

Every one of these decisions is recognisable to someone who has worked the floor, and opaque to
someone who hasn't. You have to have typed the fake date of birth under pressure to understand why
the `null`/`unknown`/`refused` distinction is not pedantry but patient safety. You have to have
watched a department go dark to understand why offline-first is not a feature but the whole point. You
have to have clicked through ten thousand confirmation dialogs to understand why the ten-thousand-and-
first one will not save anyone, and why restoring the physical affordance of possession will. You have
to have been phoned at 3 a.m. with a critical potassium to know which single push notification is
sacred and which thousand are noise. You have to have tried to fix a wrongly-merged chart in a legacy
system — and discovered it is nearly impossible — to insist that *repair* be a first-class, auditable,
reversible operation from day one.

This is why case-mining real failure modes is the most productive design activity in the project, more
than any abstract architecture debate. You take a genuine breakdown from a real shift — the patient who
was treated before they "arrived" because the ambulance crew and the ED clock disagreed; the sealed
psychiatric episode that still has to be able to warn the surgeon about an anaesthetic interaction
without disclosing its contents; the nightly imaging sync that once ground all of clinical care to a
halt — and you test whether the existing primitives absorb it without new machinery. So far they have,
which is the strongest available evidence that the foundations are the right ones. When the append-only
log, the overlay model, acknowledged uncertainty, and identity-as-claim keep absorbing real-world
catastrophes without needing to be extended, that is not luck. It is what happens when the data model
was derived from the failures in the first place, instead of from a billing schema.

A vendor cannot build this, because a vendor's deepest interest is the opposite of the mission: the
data must be a little bit trapped, the interoperability a little bit broken, the upgrade a little bit
mandatory, or the business does not work. Only a project with no vendor in the room is *free* to let
the point of care win every argument. And only clinicians — the people who have been failed by these
systems and who carry, in their hands and their memory, the precise shape of every way it broke — know
which arguments those are.

---

## The cairn

The system is named for a cairn: a hand-built stack of stones that marks the safe path through wild
country. It needs no power, no network, no infrastructure. It is built by accretion — each traveller
adds a permanent stone, and none are ever taken away. It is decentralised, raised by many hands across
a landscape, and it is found in nearly every culture on earth. Every property in that image is a
property in the architecture: append-only accretion, no central authority, resilience without
infrastructure, universality across settings, and a single humble job done reliably when everything
else has failed.

That is finally what it takes to design a fractal-topology EHR. Not cleverness for its own sake, but a
refusal to let any interest other than the patient's bedside drive a single decision — and then the
discipline to follow that refusal all the way down into the data model, the topology, the
synchronisation semantics, and the thousand small frictions of a clinical day. Build it so the grid can
go down and the chart stays up. Build it so the record would rather admit it doesn't know than tell you
a confident lie. Build it so it never wastes the one resource the clinician cannot get back, which is
time and attention at the point of care. Do those three things honestly, at every scale, in one
codebase, and you have a stone worth adding to the stack.

---

> [!NOTE] Related reading
> - The full [architecture specification](../spec/index.md) and the [decision log (ADRs)](../spec/decisions/README.md) — where the *why* behind every choice lives.
> - The mechanisms named above, in detail: [Synchronisation](../spec/sync.md) · [Identity subsystem](../spec/identity.md) · [Data model](../spec/data-model.md) · [Topology](../spec/topology.md) · [Language & substrate](../spec/language-substrate.md).
> - A companion essay for engineers: [A health record that assumes the network will fail](a-health-record-that-assumes-the-network-will-fail.md).
> - The project on [GitHub](https://github.com/cairn-ehr/cairn-ehr) — AGPL-3.0, contributions and clinical failure modes welcome.
