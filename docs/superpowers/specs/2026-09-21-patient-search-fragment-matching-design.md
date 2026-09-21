# Design — patient search matches fragments, not only whole tokens (slice 1)

- **Date:** 2026-09-21
- **Issue:** [#636](https://github.com/cairn-ehr/cairn-ehr/issues/636)
- **Blocks:** slice 2, `2026-09-20-registration-search-funnel-ui-design.md`
- **Spec sections:** §5.3 / §5.8 (search-before-create), §1.2 (paper-parity), §5.11 (read budgets)
- **ADRs:** [ADR-0061](../../spec/decisions/0061-registration-is-an-act-that-carries-its-search.md)
  (the attestation carries the query verbatim) ·
  [ADR-0014](../../spec/decisions/0014-locale-pluggable-matcher-comparators.md) (advisory matcher;
  *no data is never disagreement*)

## The problem

`db/046_patient_search.sql`'s name pass matches **exact tokens**:

```sql
CROSS JOIN LATERAL regexp_split_to_table(lower(normalize(pn.value, NFC)), '\s+') AS tok
JOIN unnest(COALESCE(p_name_tokens, ARRAY[]::text[])) t
  ON tok = lower(normalize(t, NFC))
```

A clerk will not type `Fyodorowksi-Eschenbacher`, or a full Latino compound surname, to find a
chart. They type a fragment and pick from a list. Today a fragment returns **zero**, and — because
zero is also what a genuinely absent patient returns — the clerk cannot tell "not here" from "you
typed too little". That is a §1.2 failure for *find an existing chart* and the duplicate-chart risk
ADR-0061 exists to prevent.

## What is already true, and must not be broken

**The query side already splits punctuation.** `SearchQuery::new` emits, per whitespace-delimited
word, both the whole edge-trimmed word *and* its alphanumeric parts (single characters dropped).
This is deliberate and documented: the whole-word token exists so a clerk typing a punctuated name
back exactly as printed matches an intact stored token, and so a John Doe callsign does not fragment
into pieces matching every John Doe ever registered.

**The stored side splits on whitespace only.** So `O'Brien-Smith` is stored as one token. The
asymmetry is deliberate and its consequence is stated in the file: typing `Brien` finds a chart
stored as separate words, but not one stored as `O'Brien-Smith`.

**The drift invariant is one-directional.** db/046's blocking keys mirror
`matcher/pipeline/db.py`'s, so *"a chart the sweep would pair is a chart this search finds"* —
i.e. sweep-paired ⊆ search-found. **Widening search preserves it**; narrowing would break it. Every
change below widens.

**Pass 3 is a UNION**, so extra tokens can only add advisory candidates, never remove one. The file's
own words: *"a missed candidate is the dangerous direction here, an extra one is merely something a
clerk dismisses."* That reasoning licenses both halves below.

**The query is carried verbatim into a permanent signed attestation** (`SearchAttestation.query`).
Nothing here changes `SearchQuery`'s shape, so no signed body changes.

## Two halves, sequenced

### 1a — symmetric stored-side tokenisation (no index change)

Project the alphanumeric parts of a stored punctuated token too, mirroring what the query side
already does, with the same single-character drop. `Eschenbacher` then finds
`Fyodorowksi-Eschenbacher` by **exact equality** — no index, no extension, no new latency profile.

This alone fixes compound and hyphenated surnames, which is most of the reported pain. It is
additive (UNION), it widens search only, and it reuses a tokenisation rule already written, reviewed
and justified on the query side.

**The callsign caveat is real and must be tested.** The query side drops single characters and keeps
the whole word specifically so callsigns (`unknown-ed-site1-…`) do not fragment into pieces matching
every John Doe. Splitting the *stored* side reintroduces exactly that risk from the other direction:
a stored callsign would project parts like `unknown`, `ed`, `site1`. A clerk typing `unknown` would
then surface every John Doe on the node. **So callsign-derived names are excluded from part
projection** — the whole token only. The §5.4 John Doe subsystem already distinguishes them, and
pass 3's existing comment notes callsigns are deliberately included in search but excluded from the
matcher.

### 1b — prefix matching (no index; the pass is already a scan)

`mich` → `Michaelowski`. Exact equality cannot do this at all, so an index-backed prefix match is
added as a fourth disjunct.

**No index, because there is none to lose.** Planning corrected this: `patient_name` carries **no
index at all**, and pass 3 already scans it with a lateral `regexp_split_to_table` over every value.
Tokens produced by a set-returning function cannot be indexed without materialising them into a
token table, so db/046's *"keeps the door open to an expression index"* is aspirational, not
current. `starts_with(tok, q)` therefore costs the same scan `tok = q` costs today: **1b does not
degrade the latency profile**, it changes the predicate applied after the split. A materialised
token table is a separate slice with its own reprojection cost, to be opened only if Task 5's
measurement demands it.

**Prefix, not infix, and `starts_with` not `LIKE`.** `LIKE q || '%'` would be a wildcard-injection
bug: `SearchQuery::new` trims only a word's EDGE punctuation, so an internal `%` or `_` survives
into a query token and LIKE would read it as a wildcard. `starts_with` has no escaping surface and
is what `LIKE 'x%'` optimises to. Infix (`%esch%`) needs `pg_trgm`, a new extension dependency on every node
including Pi-class ones, for a case 1a already covers in its common form (the fragment a clerk types
is usually the *start* of a name part, and after 1a each part of a compound is its own token).
Infix stays out; it can be added additively later if measurement shows it is needed.

**A minimum fragment length.** A one- or two-character prefix matches a large fraction of any
population, which inflates the advisory candidate set and, worse, would write a large candidate list
into a signed attestation if such a search ever preceded a registration. Minimum **3 characters**
for the prefix disjunct.

**This must not make a short name unfindable, and the distinction is easy to implement wrongly.**
The minimum gates the *prefix* disjunct only. Exact matching is a separate disjunct with no length
rule, and both tokenisers already preserve short whole words: `SearchQuery::new` filters `parts` to
length > 1 but applies **no length filter to `whole`**, and db/046 guards only `tok <> ''`. 1a's
part projection inherits the same single-character (not two-character) drop. So:

| Clerk types | Stored | Route | Found |
|---|---|---|---|
| `Wu` | `Wu` | exact | yes |
| `Ng` | `Ng Wei` | exact, on the whitespace-split token | yes |
| `Li` | `Li-Wong` | exact, on the 1a part `li` | yes |
| `Wu` | `Wuang` | prefix — gated | no |

Only the last row is refused, and it is the unselective case the minimum exists for. Two- and
three-character surnames — common in romanised CJK and Vietnamese names — remain **fully findable by
exact match**; they simply gain less from the fragment affordance, because for such a name the
fragment is essentially the whole name. Nothing is lost relative to today.

> **Corrected 2026-09-21 (#638).** The paragraph above reasons only about *romanised* short
> surnames, and the conclusion "the fragment is essentially the whole name" is false for names in
> CJK **script**. `李小明` is a complete name in three characters and projects exactly one token, so
> the natural gesture `李小` was gated at two characters — leaving the duplicate-chart failure this
> slice exists to fix intact for Han, Kana and Hangul, which is ADR-0014's cultural-capture shape.
> **The gate now counts BYTES (`octet_length`), not characters**, so `李` and `李小` are admitted
> while `mi` stays gated — culture-neutral because it names no script. Honest limit: 2-byte scripts
> (Cyrillic, Greek, Hebrew, Arabic) now admit a two-character prefix, an error in the safe
> direction.

**The matcher does not widen.** The invariant needs sweep-paired ⊆ search-found, and widening only
search keeps that true. Widening the matcher's blocking keys is a separate question with its own
evaluation (recall/precision, sweep cost) and is explicitly out of scope — but the DRIFT NOTE must
be updated to say that the two sides are now *deliberately* asymmetric in this one direction, rather
than leaving a future reader to think they have drifted by accident.

## Paper-parity benchmark (§1.2)

**Paper counterpart:** flipping to a section of the alphabetical patient index drawer — you read the
first few letters on the card edge, you do not read whole names.

**Steps:**

| | Find an existing chart by partial surname |
|---|---|
| Paper acts (N) | 3 — ask a name, flip to the letters, pull the card |
| Architecture-forced (M) | 2 — type a fragment, pick from the list |
| UI bundling target (K) | 2 (delivered by slice 2) |

`M ≤ N`, so no architecture defect. Today's M is effectively unbounded for a compound surname: the
clerk must type it exactly or fail, which is why this slice exists.

**Time + cognitive load.** The existing ceiling is **5 s to find an existing chart**, stated in
db/046 itself. This slice owes a measurement at realistic volume, because 1b is the change most
likely to breach it: prefix matching over a large `patient_name` projection with the wrong index is
a sequential scan. The measurement is taken against a synthetic population (the matcher's volume
generator already exists) at the largest size a Pi-class node is expected to hold, for both a
selective fragment and a deliberately unselective one. Cognitive load falls: the clerk no longer has
to reproduce punctuation and spelling exactly.

## Testing strategy

TDD; the failing test comes first. All DB-gated tests run against the SQL mirrors as well.

**1a.** A chart stored as `Fyodorowksi-Eschenbacher` is found by `Eschenbacher`, by `Fyodorowksi`,
and still by the intact whole token. A chart stored as separate words is still found (no
regression). **A stored callsign is NOT fragmented**: typing `unknown` does not surface John Doe
charts — the anti-vacuity control for the exclusion above, and the test most likely to catch a naive
implementation.

**1b.** A 3-character prefix finds a longer token; a 2-character one does not engage the prefix
disjunct. **A two-character surname is still found by exact match** (`Wu` finds `Wu`; `Li` finds
`Li-Wong` via 1a's part) — the test that separates "the minimum gates prefixes" from the wrong
implementation, "the minimum gates short tokens", which passes every other test in this suite; a prefix matches at token start but not mid-token (pinning prefix-not-infix, so a later
change to trigram is a deliberate decision rather than a drift). The index is actually used —
asserted via the plan, not assumed, since an unusable index is the whole latency risk.

**Invariant.** A property test over a generated population: every pair the matcher's sweep would
block together is still found by search. This is the DRIFT NOTE made executable, and it is what
makes "widening is safe" a checked claim rather than an argument.

**Budget.** The measurement above, recorded in the plan's §1.2 section.

## Out of scope

Infix/trigram search. Widening the advisory matcher's blocking keys. Phonetic or fuzzy matching.
Any change to `SearchQuery`, `SearchAttestation`, or any signed body. The funnel UI (slice 2).

## Risks

- **1b's index may not hold the 5 s budget** at realistic volume. If it does not, 1a still ships and
  1b becomes its own problem with measurements attached, rather than an unbounded regression.
- **`normalize(…, NFC)` inside the indexed expression** must be identical on both sides or the index
  is silently unused. The expression is already shared by copy; this slice should make that sharing
  explicit rather than leave two literals to drift.
- **Part projection enlarges `patient_name`'s effective key set**, so the sweep's own cost profile
  may move even though its keys do not. Worth watching, not blocking.
