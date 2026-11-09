//! The allergy list's node side (allergy slices 1 and 2): sign an entry, a "none known"
//! statement, a de-label or a withdrawal under a human's key (`sign`), read every allergy row of a
//! patient's linked charts (`read`), fold the rows into threads (`thread`) and word them for the
//! strip and the expanded list (`view`). The pure wire builders are `cairn_event::allergy`'s and
//! `cairn_event::allergy_overlay`'s; the floor is db/074 and db/075.
pub mod read;
pub mod sign;
mod strip;
#[cfg(test)]
mod test_fixtures;
pub mod thread;
pub mod view;
