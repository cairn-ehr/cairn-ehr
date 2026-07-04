# ADR-0041 — The progress-note narrative format: one signed event, markdown narrative, and manifest-keyed media anchors

- **Status:** Accepted
- **Date:** 2026-07-04
- **Refines:** [ADR-0012](0012-schema-evolution-event-format-and-legibility-across-time.md) (legibility twin),
  [ADR-0013](0013-attachments-content-addressed-lazy-blob-tier.md) (attachment reference shape),
  [ADR-0020](0020-active-write-thin-encounters-and-the-delete-vs-erase-distinction.md) (active-write model),
  [ADR-0039](0039-globalise-authored-legibility-twin.md) (authored twin)

## Context

The first **narrative clinical surface** is next on the build path: the progress note. Design work on
sessions, providers, and the PoC GUI is still ahead, so this decision is deliberately scoped to the
**wire format of the note event itself** — the shape that lets any future GUI render rich text with
inline media (images, drawn annotations, audio, and later modalities) while every principle and prior
ADR holds, and that any node decades ahead or behind can still read as plaintext. The format is a
can't-retrofit commitment of the same class as the [ADR-0013](0013-attachments-content-addressed-lazy-blob-tier.md)
attachment-reference shape: the first production `note.authored` event freezes it under a signature
forever, so it must be settled before the surface is built.

The forces:

- **A note is rich content in an append-only, set-union world.** The intuitive structure — a note as
  a **linked list of containers**, each holding text or graphics — fails principle 1 in both readings.
  A pointer-editable list requires *rewriting* an existing container's link (mutation of signed
  content, forbidden outright). An append-only chain (each container pointing at its predecessor)
  survives immutability but breaks under concurrency and partial sync: two devices appending offline
  to the same predecessor turn the list into a **tree** after set-union, with no clinically-reasoned
  merge policy for which fork is the real note order; and because set-union guarantees delivery but
  not *ordered* delivery, a node holding half the chain cannot honestly render the half it has — an
  ED handover could read a note missing its middle. A linked list puts *intra-note structure* on the
  sync plane, where it becomes exactly the dangerous-merge class the architecture exists to preclude.
- **Everything the linked list was reaching for already exists.** The chain *between* notes is the
  [ADR-0020](0020-active-write-thin-encounters-and-the-delete-vs-erase-distinction.md) encounter fold;
  continuation and correction are the overlay discipline; concurrent authorship is two events in the
  same encounter folded by `t_effective` — precisely how two clinicians writing in one paper chart
  behave, with no structural conflict possible; incremental writing (the ED note written in bursts
  over hours) is the [§3.10](../data-model.md#310-session-identity-event-authorship-and-draft-durability)
  durable draft store, which is node-local and never an event.
- **Raw data must be human-readable without preventing rich rendering** (principle 11). The body a
  clinician signed must remain legible as ink on paper for as long as it exists — including its
  media, which cannot be twinned from pixels ([ADR-0013](0013-attachments-content-addressed-lazy-blob-tier.md) point 6).
- **Rendering belongs to the edges** (principle 12). Many front-ends will render this format; the
  core may commit to *content, position, reference, and description* — never to presentation. The
  format must also be safe to hand to renderers the steward will never audit.
- **Media are the highest-stigma erasure case** (wound photography, psych-adjacent recordings —
  [ADR-0013](0013-attachments-content-addressed-lazy-blob-tier.md)): the format must allow a single
  figure to be crypto-shredded without touching the signed note.
- **Audio is not one thing.** A recording may be *content* — a patient's verbal consent or treatment
  refusal, a psychotic episode, a clinician's verbatim dictation — or *provenance*: the source
  recording an AI-scribe transcript derives from ([ADR-0007](0007-authorship-and-accountability.md)
  territory). The format must carry both roles without conflating them.

## Decision

The progress note is **one signed event whose body is a markdown narrative plus a media manifest**;
intra-note structure is **never** inter-event structure. Canonical home:
[data-model §3.19](../data-model.md#319-the-progress-note-narrative-format-one-signed-event-markdown-narrative-and-manifest-keyed-media-anchors).

1. **One note = one signed event** (`event_type = note.authored`, `schema_version = note/1`).
   Ordering is intrinsic to the body, arrival is atomic (a note can never half-arrive), and there is
   exactly one legibility twin, one signature, one attestation — the paper model, where the clinician
   signs the *entry*, not each sentence. The contributor set ([§3.9](../data-model.md#39-authorship-and-accountability))
   carries scribe-writes/physician-attests at note level, per the
   [ADR-0020](0020-active-write-thin-encounters-and-the-delete-vs-erase-distinction.md) bet that
   sub-note span-level provenance is not needed (this ADR shares that falsifier). Rejected
   alternative: the linked list / container-per-event structure, for the sync reasons in Context.

2. **The body is `narrative` + `media`.** `narrative` is a single markdown string in the pinned
   profile (point 5). `media` is an array of manifest entries, each carrying the
   [ADR-0013](0013-attachments-content-addressed-lazy-blob-tier.md) day-one attachment-reference
   shape — self-describing content digest, media type, byte length, rendition set, seal indicator /
   DEK-wrap reference, inline-vs-reference — plus a **note-local `id`** and a **mandatory non-empty
   human `descriptor`**. Structured clinical actions (orders, prescriptions) are *never* embedded in
   the note body: they are separate events rendered into the note *view* by the encounter fold
   ([ADR-0020](0020-active-write-thin-encounters-and-the-delete-vs-erase-distinction.md)); the note
   event is pure narrative.

3. **One anchor grammar for every media kind, manifest-keyed.** Inline placement uses standard
   markdown image syntax with a `cairn:` URI: `![<descriptor>](cairn:att/<id>)`. The anchor means
   *"render this manifest entry here"* — not "this is an image". The manifest's `media_type` tells a
   renderer whether to place an image, an audio player, a waveform viewer, or an honest
   *"referenced — not yet retrieved"* card; adding a future modality is a new media type, **zero
   format change**, and an older renderer degrades to the anchor's text. Anchors are keyed by
   manifest `id`, never by inline digest — a 64-hex digest in prose degrades the raw legibility
   principle 11 protects, and the digest already lives once in the manifest, covered by the same
   event signature, so the indirection costs nothing in integrity. **A dangling anchor (no matching
   manifest entry) is rejected at the validated submit floor** ([§9.6](../language-substrate.md#96-the-validated-submit-surface-the-write-path)) —
   structurally impossible to sync. A manifest entry *without* an anchor is legal: an attachment
   clipped to the note but not placed inline (paper-clip parity).

4. **The descriptor is the twin substrate.** Every manifest entry must carry an honest, non-empty
   human description (principle 4: never record media you can say nothing about). The
   [§3.13](../data-model.md#313-schema-evolution-event-format-and-the-legibility-twin) authored twin
   is then a pure mechanical derivation ([ADR-0039](0039-globalise-authored-legibility-twin.md)
   pattern): the narrative verbatim, each anchor replaced by
   `[attachment: <descriptor> — <media_type>, <size>]`, and one such line appended per non-anchored
   manifest entry. That twin is what a text-only node, a screen reader, full-text search, and the
   RAG substrate see.

5. **The markdown profile is pinned, versioned, austere, and additive-only.** Profile `note/1`:
   paragraphs, `**bold**`, `*italic*`, headings, ordered/unordered lists, blockquote, and the anchor
   grammar. **Excluded forever:** raw HTML (an injection surface for renderers the steward can never
   audit, and a legibility-across-time hazard) and load-bearing external URLs (dead links fail
   offline-first and legibility across time; media reference content-addressed digests only).
   **Excluded from v1, addable additively:** tables (a table wish is usually a modeling smell — drug
   lists and similar constructs should be *generated* from structured events, not hand-written
   inline), footnotes, code blocks. An old renderer meeting a future construct shows it as literal
   text — degraded but honest, the [§3.13](../data-model.md#313-schema-evolution-event-format-and-the-legibility-twin)
   ladder. Profile evolution rides `schema_version` under
   [ADR-0012](0012-schema-evolution-event-format-and-legibility-across-time.md) additive-only rules.

6. **Figure-granular erasure is a named property of this shape.** Because the note holds only digest
   + descriptor and the bytes are a separately-sealed blob, a per-blob DEK crypto-shred
   ([ADR-0005](0005-erasure-key-custody-and-crypto-shredding.md)/[ADR-0013](0013-attachments-content-addressed-lazy-blob-tier.md))
   kills a wound photo while the signed note stays **byte-identical and fully legible**; the anchor
   degrades to `[attachment: <descriptor> — shredded]` down the standard
   `min(retrievable, parseable, cleared)` ladder. No note rewrite, no signature break, no new
   mechanism — the strongest single argument for reference-by-manifest over any inlined-media shape.

7. **Drawn graphics are static-profile SVG with a mandatory flattened-raster rendition.** Clinicians
   draw constantly on paper (body charts, wound outlines); the drawn artifact is an SVG attachment in
   a **pinned static profile** — no script, no external href, no CSS import — enforced at the submit
   floor, because principle 12 means every future renderer cannot be audited. SVG is text, so the
   raw artifact itself stays human-inspectable. Its rendition set **must** include a flattened
   raster: the pinned *"what the clinician saw and signed"* appearance, immune to decades of SVG
   renderer drift ([ADR-0013](0013-attachments-content-addressed-lazy-blob-tier.md) point 6 doing its
   job). A drawn-on-template annotation is a stroke-overlay SVG referencing the template blob's
   digest — overlay, never edit. A tiny sketch may ride the
   [ADR-0013](0013-attachments-content-addressed-lazy-blob-tier.md) small-blob inline path.

8. **A generic `refs` field carries inter-event relationships:** `refs: [{rel, event}]`, where `rel`
   is a small **closed enum** (initially `addendum-to`, `correction-of`, `transcript-of`) evolved
   additively like the identity algebra's verb set. This resolves the audio dual role without a
   special case: a recording that *is* content (verbal consent, treatment refusal, a psychotic
   episode, verbatim dictation) is a **manifest entry**; a scribe-generated narrative whose *source*
   was a recording carries `refs: [{rel: "transcript-of", event: <the event that recorded the
   audio>}]` — provenance via reference, contributor roles via the
   [ADR-0007](0007-authorship-and-accountability.md)/[ADR-0028](0028-finalized-closed-contributor-role-enum.md)
   contributor set. Addendum and correction need no bespoke event types.

9. **Two day-one reservations** (cheap now, near-impossible later, the
   [ADR-0013](0013-attachments-content-addressed-lazy-blob-tier.md) point-5 discipline):
   - **`encounter` in the signed body from the first production note**, nullable — `null` is the
     honest *"no grouping context asserted"* (principle 4,
     [ADR-0020](0020-active-write-thin-encounters-and-the-delete-vs-erase-distinction.md) thin
     encounter), never a manufactured context. Projection columns can follow whenever; the *body
     field* is the part that must be inside the signed bytes from day one.
   - **The event-anchor grammar is reserved:** `![<text>](cairn:event/<uuid>)` — the same anchor
     grammar targeting an *event*, rendered as that event's twin at that position. Not valid in
     `note/1` (nothing to point at yet); reserving it now means the
     [ADR-0020](0020-active-write-thin-encounters-and-the-delete-vs-erase-distinction.md)
     type-through case (an order line sorting *between* two paragraphs of one narrative) lands later
     as a pure additive schema bump, never a format redesign.

10. **Blast radius ([§9](../language-substrate.md)).** **Safety-critical** (in-DB/Rust): the
    anchor↔manifest integrity check, markdown- and SVG-profile enforcement, descriptor
    non-emptiness, twin fidelity, and the digest binding (inherited from
    [ADR-0013](0013-attachments-content-addressed-lazy-blob-tier.md)) — all at the validated submit
    floor. **Fit-for-purpose:** every renderer, editor, audio player, and drawing surface. The
    austerity of point 5 is what keeps the floor validator small enough to be reviewer-legible.

## Consequences

- **Easier.** UI pluralism gets full rich-text-with-media capability while the core commits only to
  content, position, reference, and description; the authored twin, full-text search, and RAG
  substrate come nearly free (the raw body *is* the note); figure-granular erasure needs no new
  mechanism; a note arrives atomically with no new sync semantics (it is just an event); and the
  slice unblocks two recorded waiting threads — identity `reattribute` (§5.5, "waits on a
  clinical-note surface") and the §5.4 photo/marks/belongings evidence (waits on the attachment
  tier).
- **Harder / new trusted surface.** A markdown-profile validator and an SVG-static-profile validator
  become floor code — small, closed grammars by design, but they are new safety-critical surface and
  must be held to reviewer-legibility. The anchor grammar and the manifest shape join the immutable
  wire commitments: a mistake here is frozen under signatures.
- **The bet.** That a single-event note with a note-level contributor set and an austere profile
  clears real clinical narrative at paper pace. We would know it is wrong if real use routinely needs
  **sub-note span-level provenance** (the same falsifier
  [ADR-0020](0020-active-write-thin-encounters-and-the-delete-vs-erase-distinction.md) recorded), if
  clinicians genuinely need hand-authored constructs the profile excludes (rather than generated
  renderings of structured events), or if narrative bodies approach the 8 MB event admission cap
  (they should not — media bytes never ride the event).
- **Policy-neutral ([principle 9](../index.md#founding-principles-the-lens-for-every-decision)).**
  Which media types a deployment accepts, size and inline thresholds, whether clinical photography
  is sealed-by-default, and audio consent/retention rules are all policy. The format carries
  mechanism only.
