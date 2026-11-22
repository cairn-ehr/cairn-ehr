//! The window's first connect to a node on another machine (#909 slice B).
//!
//! `cairn_node::terminal_link::prepare` makes the pin at a terminal's first launch, but the node
//! admits the terminal only once its operator runs `admit-terminal` at the node, so that first
//! launch's connect is refused as not admitted. The launch then fails, and what it prints must
//! say exactly what to type at the node, with this terminal's real fingerprint: the clerk reads
//! it out to the operator, who should not have to compose it.
use std::path::Path;

use cairn_node::db::ConnectFailed;
use cairn_node::terminal_link::files::TerminalFiles;
use cairn_node::terminal_link::Link;
use tokio_postgres::Client;

/// The launch's first connect over `link`. A pinned link the node has not admitted fails the
/// launch with [`launch_refusal`]'s text; every other failure is returned as it came.
pub async fn first_connect(
    link: &Link,
    conn: &str,
    terminal_dir: Option<&Path>,
) -> anyhow::Result<Client> {
    let err = match link.connect_with_task(conn).await {
        Ok((client, _task)) => return Ok(client),
        Err(e) => e,
    };
    let (Link::Pinned(_), Some(dir)) = (link, terminal_dir) else {
        return Err(err);
    };
    let refusal = err.downcast_ref::<ConnectFailed>();
    // shortcut: told apart from the link's other refusals (another key, not a gateway) by its
    // own words, as `terminal_link::connector` writes them; a typed marker on `ConnectFailed`
    // is the upgrade if a second reader needs it.
    if !refusal.is_some_and(|f| f.refused_by_client() && f.to_string().contains("not admitted")) {
        return Err(err);
    }
    let key = TerminalFiles::at(dir).load_or_create_key()?;
    let kid = hex::encode(key.verifying_key().to_bytes());
    anyhow::bail!(launch_refusal(&format!("{err:#}"), &kid)?)
}

/// The not-admitted launch's text: the link's refusal, then the one command the node's operator
/// runs to admit this terminal (`terminal_pub_hex` is its 64-hex public key). Pure.
pub fn launch_refusal(refusal: &str, terminal_pub_hex: &str) -> anyhow::Result<String> {
    let fp = cairn_event::short_fingerprint(terminal_pub_hex)?;
    Ok(format!(
        "{refusal}\n\nAt the node, its operator runs:\n  \
         cairn-node admit-terminal {fp} --label \"<where this terminal is>\""
    ))
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The launch's refusal ends with the exact command the node's operator types, carrying this
    /// terminal's real fingerprint.
    #[test]
    fn a_not_admitted_launch_names_the_admit_command_with_this_terminals_fingerprint() {
        let (_sk, kid) = cairn_event::generate_key().unwrap();
        let fp = cairn_event::short_fingerprint(&kid).unwrap();
        let text = launch_refusal("this terminal is not admitted by the node.", &kid).unwrap();
        assert_eq!(
            text,
            format!(
                "this terminal is not admitted by the node.\n\nAt the node, its operator runs:\n  \
                 cairn-node admit-terminal {fp} --label \"<where this terminal is>\""
            )
        );
        assert!(launch_refusal("x", "not a key").is_err());
    }
}
