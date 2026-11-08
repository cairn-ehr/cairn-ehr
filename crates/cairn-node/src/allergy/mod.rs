//! The allergy list's node side (allergy slice 1): sign an entry or a "none known" statement under
//! a human's key (`sign`), and read every allergy row of a patient's linked charts (`read`). The
//! pure wire builders are `cairn_event::allergy`'s; the floor is db/074.
pub mod read;
pub mod sign;
