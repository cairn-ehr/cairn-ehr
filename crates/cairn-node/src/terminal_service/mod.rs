//! The terminal service: the node-side process that does the window's node-signed acts (#909).
//!
//! WHY IT EXISTS. #909 takes the node's signing key off every clinician's window. Two acts the
//! window used to sign with that key are node-signed by design: registering a patient
//! (ADR-0061) and the timed commit of an abandoned note draft (db/067). They move here, to one
//! process that runs beside the database and loads the key unattended, as `serve` does
//! (design D1, docs/superpowers/specs/2026-11-16-pinned-terminal-link-909-design.md).
//!
//! The window and this process meet only inside the database, through db/083's doors: the
//! window writes a registration REQUEST, this process serves it ([`registration::serve_one`]).
//! [`run::run`] is the loop that calls it and [`commit::commit_tick`]; [`startup::check`] runs
//! once before it, from the `cairn-node terminal-service` verb.

pub mod commit;
pub mod gateway;
pub mod registration;
pub mod run;
pub mod startup;
