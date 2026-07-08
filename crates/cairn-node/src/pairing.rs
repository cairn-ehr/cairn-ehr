use crate::identity::Identity;
use base64::{engine::general_purpose::STANDARD, Engine};
use cairn_event::{
    short_fingerprint, sign_pairing_bundle, verify_pairing_bundle, Hlc, PairingBundle, SigningKey,
};

/// Build a signed, base64-encoded pairing offer from this node's identity.
pub fn make_offer(id: &Identity, sk: &SigningKey, nonce: &str) -> anyhow::Result<String> {
    make_offer_for(&id.node_id_hex, &id.pubkey_hex, &id.address, nonce, sk)
}

/// Build a signed, base64-encoded pairing offer from raw fields (used in tests
/// to construct "node B's" offer without a live DB).
pub fn make_offer_for(
    node_id_hex: &str,
    pubkey_hex: &str,
    address: &str,
    nonce: &str,
    sk: &SigningKey,
) -> anyhow::Result<String> {
    let b = PairingBundle {
        node_id_hex: node_id_hex.into(),
        pubkey_hex: pubkey_hex.into(),
        address: address.into(),
        fingerprint: short_fingerprint(pubkey_hex)?,
        nonce: nonce.into(),
        hlc: Hlc {
            wall: 0,
            counter: 0,
            node_origin: node_id_hex.into(),
        },
    };
    Ok(STANDARD.encode(sign_pairing_bundle(&b, sk)?))
}

/// Decode and verify a base64-encoded pairing offer, returning the [`PairingBundle`].
/// Signature and self-consistency of the fingerprint are both checked.
pub fn read_offer(b64: &str) -> anyhow::Result<PairingBundle> {
    let raw = STANDARD.decode(b64.trim())?;
    Ok(verify_pairing_bundle(&raw)?)
}
