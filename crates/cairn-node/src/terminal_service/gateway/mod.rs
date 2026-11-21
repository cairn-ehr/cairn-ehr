//! The terminal gateway: the LAN door through which a remote window reaches Postgres (#909 slice B).
//!
//! WHY IT EXISTS. The window no longer holds the node's key, so it can sit on another machine. It
//! still speaks Postgres's own protocol; the gateway is the only thing standing between the LAN and
//! the database, and it forwards bytes rather than re-implementing Postgres (design D2,
//! docs/superpowers/specs/2026-11-16-pinned-terminal-link-909-design.md).
//!
//! D2's five steps, in order, for each connection:
//! 1. Read the 8-byte opening message. An `SSLRequest` gets `S` and a TLS handshake; a
//!    `GSSENCRequest` gets `N` and must then be followed by an `SSLRequest`. Anything else
//!    (a plain-text startup or cancel included) is closed without a reply.
//! 2. TLS 1.3 only, mutual. The server certificate is the node's identity; the client
//!    certificate's Ed25519 key must be `active` in the terminal trust set. No fallback verifier.
//! 3. Inside TLS, read the `StartupMessage` (or a `CancelRequest`, forwarded as is). Forward a
//!    startup only for a login that belongs to `cairn_window`, is not over-powered, and names the
//!    node's own database; anything else gets a Postgres-shaped `ErrorResponse` naming no role.
//! 4. Open a connection to Postgres by the route the local window already uses, replay the
//!    startup, and pipe both directions until either side closes. SCRAM still runs end to end.
//! 5. TCP keepalive on the LAN side, so a vanished terminal's session ends and frees its locks.
//!
//! This module tree is built up in tasks; today it holds only [`wire`], the pure byte-level
//! parsing and encoding every later step uses. Further submodules are added by later tasks.

pub mod wire;
