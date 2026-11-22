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
    if !refusal.is_some_and(|f| f.refused_by_client() && says_not_admitted(&f.to_string())) {
        return Err(err);
    }
    // The extra line is a courtesy on top of the refusal: if this terminal's key cannot be read
    // back (never MADE here: `load_key`), the refusal itself is what the launch reports.
    let with_command = TerminalFiles::at(dir).load_key().and_then(|key| {
        launch_refusal(
            &format!("{err:#}"),
            &hex::encode(key.verifying_key().to_bytes()),
        )
    });
    match with_command {
        Ok(text) => Err(anyhow::anyhow!(text)),
        Err(_) => Err(err),
    }
}

/// Is `text` the link's not-admitted refusal (`terminal_link::connector::not_admitted_text`)
/// rather than one of its other refusals (another key, not a gateway) or a login's? Pure.
///
/// shortcut: told apart by the link's own words, pinned against the real sentence by
/// `the_not_admitted_refusal_is_recognised_by_the_links_own_words`; a typed marker on
/// `ConnectFailed` is the upgrade if a second reader needs it.
fn says_not_admitted(text: &str) -> bool {
    text.contains("is not admitted by the node")
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

    /// The launch tells the not-admitted refusal apart by its words, so it is tested against the
    /// REAL sentence the link writes (`cairn_node`'s own), not a copy: a reword there that this
    /// predicate no longer recognises fails here instead of silently dropping the admit line.
    #[test]
    fn the_not_admitted_refusal_is_recognised_by_the_links_own_words() {
        let (_sk, kid) = cairn_event::generate_key().unwrap();
        let fp = cairn_event::short_fingerprint(&kid).unwrap();
        let real = cairn_node::terminal_link::connector::not_admitted_text(&fp);
        assert!(says_not_admitted(&real), "{real}");
        for other in [
            "connecting to the database: password authentication failed [28P01]",
            "connecting to the database: invalid configuration: password missing",
        ] {
            assert!(!says_not_admitted(other), "{other}");
        }
    }
}
