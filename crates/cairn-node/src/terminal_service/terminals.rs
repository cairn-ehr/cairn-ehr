//! The owner's console verbs for the terminal trust set (#909 slice B): `admit-terminal`,
//! `revoke-terminal`, `list-terminals`.
//!
//! WHY. The gateway admits only terminals whose key is `active` in `terminal_trust`. A terminal
//! that dials in is recorded `pending`; the operator reads its fingerprint aloud at the terminal,
//! then admits it here. Approval is the OWNER's act (db/084's doors), so these verbs run as the
//! owner. All the rules live in the doors; this file only normalises what the operator typed,
//! calls them, and renders the answer.
use tokio_postgres::Client;

/// One `terminal_trust` row as the operator sees it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TerminalRow {
    pub fingerprint: String,
    pub status: String,
    pub label: Option<String>,
    pub last_seen: String,
}

/// Map what an operator typed (any letter case, dashes or none) onto the stored form: five
/// upper-case 4-hex groups joined by `-`. Anything but 20 hex digits is an `Err`. Pure.
pub fn normalise_fingerprint(s: &str) -> Result<String, String> {
    let hex: String = s.trim().chars().filter(|c| *c != '-').collect();
    if hex.len() != 20 || !hex.chars().all(|c| c.is_ascii_hexdigit()) {
        return Err(format!(
            "a terminal fingerprint is 20 hex digits (e.g. ABCD-0123-4567-89AB-CDEF); got {s:?}"
        ));
    }
    let hex = hex.to_ascii_uppercase();
    // Every char is ASCII, so slicing on byte offsets is safe.
    Ok((0..5)
        .map(|i| &hex[i * 4..i * 4 + 4])
        .collect::<Vec<_>>()
        .join("-"))
}

/// One line per terminal, in the order given (the caller passes newest first). Pure.
pub fn render(rows: &[TerminalRow]) -> String {
    if rows.is_empty() {
        return "no terminal has tried to connect yet".to_string();
    }
    rows.iter()
        .map(|r| {
            format!(
                "{}  {:<8}  {}  last seen {}",
                r.fingerprint,
                r.status,
                r.label.as_deref().unwrap_or("(no label)"),
                r.last_seen
            )
        })
        .collect::<Vec<_>>()
        .join("\n")
}

/// A database failure (a door's refusal, or a failed read) as the operator should read it: its
/// own message, never `db error`.
fn door_refusal(e: tokio_postgres::Error) -> anyhow::Error {
    anyhow::anyhow!(super::commit::primary_message(&e.into()))
}

/// Admit a pending terminal under `label`. The door's refusal (unknown, ambiguous, not pending,
/// no label) is returned as its own error, reaching the operator through `primary_message`.
pub async fn admit(db: &Client, fingerprint: &str, label: &str) -> anyhow::Result<String> {
    let fp = normalise_fingerprint(fingerprint).map_err(|e| anyhow::anyhow!(e))?;
    db.query_one("SELECT cairn_terminal_admit($1, $2)", &[&fp, &label])
        .await
        .map_err(door_refusal)?;
    Ok(format!(
        "terminal {fp} admitted as {label:?}; it can connect from its next attempt"
    ))
}

/// Revoke a terminal (final; revoking an already revoked one is a no-op).
pub async fn revoke(db: &Client, fingerprint: &str) -> anyhow::Result<String> {
    let fp = normalise_fingerprint(fingerprint).map_err(|e| anyhow::anyhow!(e))?;
    db.query_one("SELECT cairn_terminal_revoke($1)", &[&fp])
        .await
        .map_err(door_refusal)?;
    Ok(format!(
        "terminal {fp} revoked; its open sessions close within 2 s and any statement it is \
         running is cancelled"
    ))
}

/// Every terminal, most recently seen first.
pub async fn list(db: &Client) -> anyhow::Result<Vec<TerminalRow>> {
    let rows = db
        .query(
            "SELECT fingerprint, status, label, last_seen::text FROM terminal_trust \
             ORDER BY last_seen DESC, fingerprint",
            &[],
        )
        .await
        .map_err(door_refusal)?;
    Ok(rows
        .iter()
        .map(|r| TerminalRow {
            fingerprint: r.get(0),
            status: r.get(1),
            label: r.get(2),
            last_seen: r.get(3),
        })
        .collect())
}

#[cfg(test)]
mod tests {
    use super::*;

    const GOOD: &str = "ABCD-0123-4567-89AB-CDEF";

    #[test]
    fn normalise_accepts_any_case_and_dashing() {
        assert_eq!(
            normalise_fingerprint("abcd-0123-4567-89ab-cdef").unwrap(),
            GOOD
        );
        assert_eq!(normalise_fingerprint("abcd0123456789abcdef").unwrap(), GOOD);
        assert_eq!(normalise_fingerprint(GOOD).unwrap(), GOOD);
    }

    #[test]
    fn normalise_refuses_wrong_length_or_non_hex() {
        assert!(normalise_fingerprint("abcd0123456789abcde").is_err()); // 19
        assert!(normalise_fingerprint("abcd0123456789abcdef0").is_err()); // 21
        assert!(normalise_fingerprint("abcd0123456789abcdeg").is_err());
        assert!(normalise_fingerprint("").is_err());
    }

    #[test]
    fn render_says_so_when_empty_and_one_line_per_row() {
        assert!(render(&[]).contains("no terminal has tried to connect yet"));
        let rows = vec![
            TerminalRow {
                fingerprint: GOOD.into(),
                status: "active".into(),
                label: Some("front desk".into()),
                last_seen: "2026-11-22 10:00:00+00".into(),
            },
            TerminalRow {
                fingerprint: "1111-2222-3333-4444-5555".into(),
                status: "pending".into(),
                label: None,
                last_seen: "2026-11-21 09:00:00+00".into(),
            },
        ];
        let out = render(&rows);
        assert_eq!(out.lines().count(), 2);
        assert!(out.lines().next().unwrap().contains("front desk"));
        assert!(out.contains("pending"));
    }
}
