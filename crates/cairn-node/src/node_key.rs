//! The node's key, as far as a clinical write needs it (#909).
//!
//! WHY: almost every write a clinician's window makes is HUMAN-signed (`apply_author(Some(..))`):
//! the node's own key never signs it. The node contributes only its public id (its `kid`, the hex
//! of its verifying key) as the `recorded` contributor. So the write paths take a [`NodeKey`]:
//! either the full key (CLI, the node's own service) or just the public id (a process that must
//! not hold the secret). A path that really needs the node to sign asks [`NodeKey::signer`],
//! which refuses with a legible message when only the id is held.

use anyhow::anyhow;
use cairn_event::SigningKey;

/// Refusal text when an act needs the node's secret key but only its public id is held.
pub const NODE_KEY_NOT_HELD: &str = "this act needs the node's own signing key, which this window does not hold; it runs on the node's terminal service";

/// The node's key: the whole thing, or only its public id (hex of the verifying key).
#[derive(Clone, Copy)]
pub enum NodeKey<'a> {
    Full(&'a SigningKey),
    Public(&'a str),
}

impl<'a> NodeKey<'a> {
    /// The node's kid (hex of its verifying key) — available from either arm.
    pub fn kid(&self) -> String {
        match self {
            NodeKey::Full(sk) => hex::encode(sk.verifying_key().to_bytes()),
            NodeKey::Public(k) => (*k).to_string(),
        }
    }

    /// The secret key, for the few acts the node itself must sign; `Public` refuses.
    pub fn signer(&self) -> anyhow::Result<&'a SigningKey> {
        match self {
            NodeKey::Full(sk) => Ok(sk),
            NodeKey::Public(_) => Err(anyhow!(NODE_KEY_NOT_HELD)),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use cairn_event::SigningKey;

    // House rule 6: derive the key at runtime, never write key bytes as a literal.
    fn key() -> SigningKey {
        SigningKey::from_bytes(&std::array::from_fn(|i| {
            (i as u8).wrapping_mul(7).wrapping_add(3)
        }))
    }

    #[test]
    fn public_key_gives_its_kid_and_refuses_to_sign() {
        let k = "ab".repeat(32);
        let n = NodeKey::Public(&k);
        assert_eq!(n.kid(), k);
        let err = n.signer().map(|_| ()).unwrap_err();
        assert_eq!(err.to_string(), NODE_KEY_NOT_HELD);
    }

    #[test]
    fn full_key_kid_matches_verifying_key() {
        let sk = key();
        let n = NodeKey::Full(&sk);
        assert_eq!(n.kid(), hex::encode(sk.verifying_key().to_bytes()));
        assert!(n.signer().is_ok());
    }
}
