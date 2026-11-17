//! The terminal service checks itself before its loop starts (#909, final review F4).
//!
//! WHY. The service is unattended: a misconfigured one would print "running" and then fail
//! quietly for ever, while every clerk's registration timed out ("the terminal service did not
//! answer") and no draft went in. Two mistakes are cheap to catch at startup and costly after:
//! a key that is not the node's (every event it signs would be refused by the floor, or worse,
//! signed by the wrong identity), and a login that cannot claim requests (a window login, or the
//! owner's role mistyped). Each refusal says what to do. The verdicts are pure; [`check`] reads.
use cairn_event::SigningKey;
use tokio_postgres::Client;

use crate::node_key::NodeKey;

/// Refuse a loaded key that is not this node's own signing key. Kids are public keys, so the
/// refusal may name both. **Pure.**
pub fn key_verdict(loaded_kid: &str, node_kid: &str) -> Result<(), String> {
    if loaded_kid == node_kid {
        return Ok(());
    }
    Err(format!(
        "The key loaded (kid {loaded_kid}) is not this node's signing key (kid {node_kid}). \
         Start the terminal service with the node's own key (--key, as serve uses)."
    ))
}

/// Refuse a login that cannot claim registration requests. **Pure.**
pub fn grant_verdict(can_claim: bool) -> Result<(), String> {
    if can_claim {
        return Ok(());
    }
    Err(
        "This login cannot serve registration requests (it has no EXECUTE on \
         cairn_registration_claim()). At the node, run cairn-node provision-terminal-service-role \
         with the owner's connection, and start the terminal service as the login it makes."
            .to_string(),
    )
}

/// Whether the connected login may run db/083's claim door.
pub async fn can_claim(db: &Client) -> anyhow::Result<bool> {
    let row = db
        .query_one(
            "SELECT has_function_privilege('cairn_registration_claim()', 'execute')",
            &[],
        )
        .await?;
    Ok(row.get(0))
}

/// Both checks on `db`, the grant first (it needs no node identity). Returns the node's origin
/// (its node id), which the loop stamps on every clock tick.
pub async fn check(db: &Client, sk: &SigningKey) -> anyhow::Result<String> {
    grant_verdict(can_claim(db).await?).map_err(anyhow::Error::msg)?;
    let node = crate::identity::load_local(db).await?;
    key_verdict(&NodeKey::Full(sk).kid(), &node.pubkey_hex).map_err(anyhow::Error::msg)?;
    Ok(node.node_id_hex)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_nodes_own_key_passes_and_another_is_refused() {
        assert!(key_verdict("ab12", "ab12").is_ok());
        let e = key_verdict("ab12", "cd34").unwrap_err();
        assert!(
            e.contains("ab12") && e.contains("cd34") && e.contains("--key"),
            "{e}"
        );
    }

    #[test]
    fn a_login_that_cannot_claim_is_refused_naming_the_owner_step() {
        assert!(grant_verdict(true).is_ok());
        let e = grant_verdict(false).unwrap_err();
        assert!(e.contains("provision-terminal-service-role"), "{e}");
    }
}
