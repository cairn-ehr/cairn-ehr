//! The at-rest key-file layer: how a Cairn node's key material lives on disk.
//!
//! WHY THIS IS ITS OWN CRATE (issue #503). Two binaries must agree on the shape of a
//! node's key files. `cairn-node` provisions them; `cairn-sync` must LOAD the unwrap
//! secret those files hold, because since
//! [ADR-0066](../../../docs/spec/decisions/0066-identity-dies-with-the-disk-custody-must-not.md)
//! that secret is an INDEPENDENT key, no longer derivable from the node's signing seed.
//! Before this crate existed the format and its loader lived inside `cairn-node`, so
//! `cairn-sync` could not read them without depending on a whole node application — and
//! it therefore kept deriving a secret that no longer matched, which stopped federated
//! sync dead.
//!
//! The three modules are one layer, split by responsibility:
//!
//! - [`seal`] — the `CAIRNK1` sealed-bundle FORMAT: Argon2id key-encryption keys derived
//!   from two independent secrets (an operational passphrase and a paper recovery code),
//!   XChaCha20-Poly1305 over the 32-byte payload, CBOR on disk.
//! - [`keystore`] — the FILES that format is written to: the signing key and its
//!   `<key>.unwrap` custody sibling, with sealed-vs-plaintext auto-detection.
//! - [`fsio`] — the crash-safe write underneath both: temp sibling, fsync, rename, fsync
//!   the directory. A torn key file is unrecoverable key loss, so this is not optional.
//!
//! A defect anywhere here is silent key loss or a forged identity, so this is §9
//! safety-critical code: pure functions wherever entropy allows, exhaustively unit-tested,
//! and optimised for a reviewer's eye over cleverness.

pub mod fsio;
pub mod keystore;
pub mod seal;
