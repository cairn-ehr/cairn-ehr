//! The text progress note (`clinical.note.asserted`, ADR-0041, sign-in slice 3a) — the node side
//! the window calls. Five small parts, each a file:
//!
//! * `draft` — the text being typed, one part per writer (db/066). It lives in the one
//!   deliberately MUTABLE store (db/065/066), reachable only through definer doors, and is never
//!   synced or signed.
//! * `whos_here` — the staff a terminal may name as the person writing (a claim, never a sign-in).
//! * `sign` — turns a draft into signed, sealed note events — one per writer's non-blank part,
//!   each later part an addendum to the one before — in ONE transaction that spends the draft,
//!   so a draft can never be both a note and still open.
//! * `read` — a chart's notes (over every linked chart, as the medication list reads, ADR-0076)
//!   with each one's authorship grade, plus the chart's open drafts.
//! * `view` — the pure words the window shows: who wrote and who signed, and when the note applies.
//!
//! The pure wire builders (payload, twin, markup escaping) are `cairn_event::note`'s.
pub mod draft;
pub mod read;
pub mod sign;
pub mod view;
pub mod whos_here;
