//! The Cairn clinical-plane sync protocol, its framing, and the transport seam.
//!
//! # Why this crate exists
//!
//! `cairn-sync` owns the clinical event log and the pull loop; `cairn-node` owns backup and
//! restore. From slice 2b (#500) a backup MEDIUM is a peer — addressed through the same
//! request/response protocol a network peer is, which is what makes ADR-0026 decision 2's
//! "backup is a configuration of the sync daemon" true rather than aspirational. Both binaries
//! therefore need these types, and `cairn-sync` is binary-only and dev-depends on `cairn-node`,
//! so it cannot export them. Same shape, same reason, as `cairn-keystore` (#503).
//!
//! # Module map
//!
//! - `wire` — `Request` / `EventsResponse`. Moved verbatim from `cairn-sync`.
//! - `framing` — the `[u32 len][payload]` frame and the clinical plane's 64 MiB cap.
//! - `transport` — `Transport`, the one seam, and its error taxonomy.
//! - `tcp` — a network peer. Today's connect/retry behaviour, moved verbatim.
//! - `medium` — a CAIRNB3 backup medium answering as a peer.
//! - `page` — the paging contract: the default page size and the termination rule.
//!
//! `transport`, `tcp`, `medium` and `page` arrive in later tasks of this slice; this task
//! (Task 1) moves only `wire` and `framing` out of `cairn-sync`, verbatim.
//!
//! # Scope today
//!
//! This crate does no database work. It does not decide WHAT goes on a medium and it does not
//! write one — `cairn-node`'s `backup.rs` still reads `node_event` and nothing else, which is
//! #500 and is NOT fixed by this crate existing. The medium gains clinical events in slice 2c
//! and gives them back in slice 2d.

mod framing;
mod medium;
mod page;
mod tcp;
mod transport;
mod wire;

pub use framing::{read_frame, write_frame, MAX_FRAME_BYTES};
pub use medium::MediumTransport;
pub use page::{page_decision, PageDecision, DEFAULT_PAGE_EVENTS};
pub use tcp::TcpTransport;
pub use transport::{Transport, TransportError};
pub use wire::{EventsResponse, Request};
