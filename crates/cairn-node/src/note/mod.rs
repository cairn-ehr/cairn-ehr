//! The text progress note (`clinical.note.asserted`, ADR-0041, sign-in slice 3a) — the node side
//! the window calls. Eleven small parts, each a file:
//!
//! * `draft` — the text being typed, one part per writer (db/066). It lives in the one
//!   deliberately MUTABLE store (db/065/066), reachable only through definer doors, and is never
//!   synced or signed.
//! * `whos_here` — the staff a terminal may name as the person writing (a claim, never a sign-in).
//! * `sign` — turns a draft into signed, sealed note events — one per writer's non-blank part,
//!   each later part an addendum to the one before — in ONE transaction that spends the draft,
//!   so a draft can never be both a note and still open.
//!   An addendum's sign also vouches for the signer's own earlier unsigned parts (`sign_vouch`).
//! * `commit` — the quarantine timer: a draft left untouched for the site's quarantine goes into
//!   the record UNSIGNED, as node-signed note events, one per non-blank part (slice 3b-2a).
//! * `discard` — throws away a draft the discarder alone wrote, as ONE signed
//!   `clinical.note-discard.asserted` recording what it said (slice 3b-2b); never a note.
//! * `read` — a chart's notes (over every linked chart, as the medication list reads, ADR-0076)
//!   with each one's authorship grade, plus the chart's open drafts.
//! * `compound` — groups a chart's notes into compound notes, each note with all its later parts
//!   (addenda, and the later writers' parts of one signed draft), so any UI can show one note
//!   where paper has one entry (slice 3c-1). Pure.
//! * `view` — the pure words the window shows: who wrote and who signed, and when the note applies.
//! * `vouch_read` — each part's signer and its later, separately signed vouches (slice 3c-2),
//!   read with the notes, only when verified and naming the part's own bytes.
//! * `vouch` — who has signed what (does the note still need a sign-off, who signed it off),
//!   and "Sign off this note": one vouch per part the presser has not signed (slice 3c-2).
//!
//! The pure wire builders (payload, twin, markup escaping) are `cairn_event::note`'s.
pub mod commit;
pub mod compound;
pub mod discard;
pub mod draft;
pub mod read;
pub mod sign;
mod sign_vouch;
pub mod supervision;
pub mod view;
pub mod vouch;
pub mod vouch_read;
pub mod whos_here;
