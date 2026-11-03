//! The text progress note (`clinical.note.asserted`, ADR-0041, sign-in slice 3a) — the node side
//! the window calls. Four small parts, each a file:
//!
//! * `draft` — the text being typed. It lives in the one deliberately MUTABLE store (db/065),
//!   reachable only through definer doors, and is never synced or signed.
//! * `sign` — turns a draft into a signed, sealed note in ONE transaction that spends the draft,
//!   so a draft can never be both a note and still open.
//! * `read` — a chart's notes (over every linked chart, as the medication list reads, ADR-0076)
//!   with each one's authorship grade, plus the chart's open drafts.
//! * `view` — the pure words the window shows: who signed, and when the note applies.
//!
//! The pure wire builders (payload, twin, markup escaping) are `cairn_event::note`'s.
pub mod draft;
pub mod read;
pub mod sign;
pub mod view;
