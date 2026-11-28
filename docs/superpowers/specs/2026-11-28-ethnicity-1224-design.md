# Ethnicity in demographics (#1224) — design

Approved by the maintainer 2026-11-28.

## Intent

A patient's ethnicity is recordable in demographics, so an advisory actor (ReportTriage) can key a
risk-stratification limit to it — today the eGFR ≤ 45 early-look limit for a patient who identifies as
Aboriginal and/or Torres Strait Islander (CARI; Med J Aust 2023, doi:10.5694/mja2.52114). Nothing
supplies the status now, so that limit changes no result.

## Decisions (maintainer's)

- **One field, `ethnicity`.** Ethnicity encompasses Indigenous identity; "Aboriginal and Torres Strait
  Islander" is one more ethnicity, not a separate field or a competing value.
- **Patient-stated first.** Self-identification is authoritative. It is the patient's responsibility to
  bear the consequences of risk stratification according to their stated ethnicity — so a patient's
  later decline displaces their earlier answer, and no other source ever displaces a patient-stated one.

## Approaches considered

- A. Indigenous status only, as its own field. Rejected: Indigenous identity is ethnicity.
- B. Two fields (ethnicity + Indigenous status). Rejected for the same reason.
- **C. One `ethnicity` field, codings open (chosen).**

Within C, the unit of an assertion:

- **An assertion is the patient's whole answer (chosen, Claude's call, approved).** Several ethnicities
  sit side by side inside one answer; only a newer answer supersedes an older one.
- Rejected: one assertion per value, projected as a set union (as identifiers are). A wrongly recorded
  value could then never be removed and would steer ReportTriage forever.

## The design

- **Event:** the existing `demographic.field.asserted`, `field = 'ethnicity'`. No new event type.
- **`value`:** the answer in plain words (the §4.5 twin's value-core), e.g.
  `"Aboriginal and Torres Strait Islander; Greek"`.
- **`facets.codings`** *(optional)*: an array of `{system, code, display}`. `system` is open text; no
  jurisdiction's vocabulary enters the core. A free-text answer with no codings is recordable
  (principle 4).
- **Floor (culture-neutral):** the generic checks already applied (non-empty field, provenance, value);
  when `codings` is present it is an array whose elements are objects with a non-empty string `display`,
  and `system`/`code` non-empty strings when present. The floor never checks a vocabulary.
- **New winner ordering, `self-identified`:** the key is
  `(provenance = 'patient-stated', HLC, provenance_rank, origin, value, content_address)` — a
  patient-stated answer beats every other provenance (document-verified included); newest wins among
  patient-stated; with none patient-stated, newest of the rest wins. A total order, as the other two.
- **Non-answers:** "declined to say" is an answer with no codings — recorded, distinct from absence.
  "Not asked" is absence: no assertion, no row.
- **Reading:** projects into `patient_demographic` (value, facets.codings, provenance, HLC), which
  `cairn_agent` already holds SELECT on. No new read door or grant.
- **Matching:** never weighs for or against a link.
- **Visibility:** shown like other demographics; not sealed.
- **Writing:** a new fact on the #1211 demographics door (`patient::demographic::assert_demographics`),
  and its CLI flag.
- **Records:** a new ADR for the `self-identified` ordering and the field; a §4.2 table row.

## Non-goals

- The gender-identity projection: spec prose says patient-stated authoritative, code is pure
  recency-first. Filed as #1245; not changed here.
- Pregnancy status (a separate request).
- Any jurisdiction's value set (the Australian standard's codes are not quoted; unverified).

## Left to the builder

Names of the Rust fact variant and CLI flag; the twin wording; which db file carries the change (a new
numbered file per the additive-migration convention).
