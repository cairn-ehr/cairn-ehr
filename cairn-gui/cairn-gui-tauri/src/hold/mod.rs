//! The window's withhold and release gestures (#832): which thread a "W" is written on and
//! which holds a Release ends, decided by pure functions in [`plan`] before anything touches
//! the database.
pub(crate) mod plan;
