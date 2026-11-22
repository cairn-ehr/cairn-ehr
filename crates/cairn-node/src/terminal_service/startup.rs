//! The terminal service checks itself before its loop starts (#909, final review F4).
//!
//! WHY. The service is unattended: a misconfigured one would print "running" and then fail
//! quietly for ever, while every clerk's registration timed out ("the terminal service did not
//! answer") and no draft went in. Two mistakes are cheap to catch at startup and costly after:
//! a key that is not the node's (every event it signs would be refused by the floor, or worse,
//! signed by the wrong identity), and a login that cannot claim requests (a window login, or the
//! owner's role mistyped). Each refusal says what to do. The verdicts are pure; [`check`] reads.
//! With `--gateway-listen`, [`check_gateway`] also refuses a login that cannot read the terminal
//! trust set and a node with no recorded gateway route (#909 slice B).
use cairn_event::SigningKey;
use tokio_postgres::Client;

use super::gateway::admit::read_login_facts;
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

/// With `--gateway-listen`: refuse a login that cannot read the terminal trust set (and the
/// gateway record), then a node with no recorded gateway route. Each names the owner's step.
/// **Pure.**
pub fn gateway_verdict(can_read: bool, has_route: bool) -> Result<(), String> {
    if !can_read {
        return Err(
            "This login cannot run the terminal gateway (it needs SELECT on terminal_trust and \
             terminal_gateway, EXECUTE on cairn_terminal_attempt, and to read login facts). At the \
             node, run cairn-node provision-terminal-service-role with \
             the owner's connection, and start the terminal service as the login it makes."
                .to_string(),
        );
    }
    if !has_route {
        return Err(
            "No gateway route is recorded, so --gateway-listen has nowhere to forward to. As the \
             owner, run cairn-node gateway-setup --upstream <127.0.0.1:5432 or the socket path>."
                .to_string(),
        );
    }
    Ok(())
}

/// [`gateway_verdict`] on `db`. The route is read only once the grant is known to be there.
pub async fn check_gateway(db: &Client) -> anyhow::Result<()> {
    // Every grant the gateway uses at run time, checked once here: missing at run time, each
    // would surface only as refusals the window retries for ever (a facts read that fails in the
    // database is judged transient, `admit::judge`).
    let granted: bool = db
        .query_one(
            "SELECT has_table_privilege('terminal_trust', 'SELECT') \
                AND has_table_privilege('terminal_gateway', 'SELECT') \
                AND has_function_privilege('cairn_terminal_attempt(text)', 'EXECUTE')",
            &[],
        )
        .await?
        .get(0);
    let me: String = db.query_one("SELECT current_user::text", &[]).await?.get(0);
    let read = if granted {
        read_login_facts(db, &me).await.map(drop)
    } else {
        Ok(())
    };
    let can_read = granted && read.is_ok();
    let has_route = can_read
        && db
            .query_one("SELECT EXISTS (SELECT 1 FROM terminal_gateway)", &[])
            .await?
            .get::<_, bool>(0);
    with_read_cause(gateway_verdict(can_read, has_route), read)
}

/// [`gateway_verdict`]'s answer, with the login-facts read's own error kept under the refusal
/// when that read is what failed: the provisioning advice is right for a missing grant, but a
/// dropped connection or a missing relation must still be visible (`{:#}` prints both). **Pure.**
pub fn with_read_cause(
    verdict: Result<(), String>,
    read: anyhow::Result<()>,
) -> anyhow::Result<()> {
    match (verdict, read) {
        (Ok(()), _) => Ok(()),
        (Err(v), Err(e)) => Err(e.context(v)),
        (Err(v), Ok(())) => Err(anyhow::Error::msg(v)),
    }
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
    fn a_gateway_needs_the_trust_grant_then_a_recorded_route() {
        assert!(gateway_verdict(true, true).is_ok());
        let e = gateway_verdict(false, true).unwrap_err();
        assert!(
            e.contains("terminal_trust") && e.contains("provision-terminal-service-role"),
            "{e}"
        );
        // The grant is reported first: without it the route cannot even be read.
        let e = gateway_verdict(false, false).unwrap_err();
        assert!(e.contains("provision-terminal-service-role"), "{e}");
        let e = gateway_verdict(true, false).unwrap_err();
        assert!(e.contains("gateway-setup") && e.contains("owner"), "{e}");
    }

    /// A facts read that FAILED keeps its own error under the verdict: a dropped connection or a
    /// missing relation must not read as "provision the role" and nothing more (review SF5).
    #[test]
    fn a_failed_facts_read_keeps_its_cause_under_the_verdict() {
        let e = with_read_cause(
            gateway_verdict(false, true),
            Err(anyhow::anyhow!("connection closed")),
        )
        .unwrap_err();
        let e = format!("{e:#}");
        assert!(e.contains("provision-terminal-service-role"), "{e}");
        assert!(e.contains("connection closed"), "the cause is kept: {e}");
        assert!(with_read_cause(gateway_verdict(true, true), Ok(())).is_ok());
        let e = with_read_cause(gateway_verdict(true, false), Ok(())).unwrap_err();
        assert!(e.to_string().contains("gateway-setup"), "{e}");
    }

    #[test]
    fn a_login_that_cannot_claim_is_refused_naming_the_owner_step() {
        assert!(grant_verdict(true).is_ok());
        let e = grant_verdict(false).unwrap_err();
        assert!(e.contains("provision-terminal-service-role"), "{e}");
    }
}
