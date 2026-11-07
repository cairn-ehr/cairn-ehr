//! The Rust side of the supervision doors (db/072): who supervises whom, who must have every signed
//! note co-signed, and who may assign supervision. The window calls [`act`] and [`staff`]; the
//! console verbs `supervision-set` / `supervision-list` call [`console_act`] and [`staff`].
//!
//! Same shape as `vault_admin::set_enroller`: the prior and the address to sign come FROM THE
//! DATABASE (so staff-ID normalisation has one home), the approval is a signed attestation over
//! that address, and the write is wrapped in [`LocalDbFault`] so a refusal (nothing written) can be
//! told from a lost answer.
use anyhow::Context;
use cairn_event::EventSigner;
use tokio_postgres::Client;

use crate::db_diagnosis::LocalDbFault;
use crate::vault_admin::approve;

/// The six supervision acts, by the exact words the doors take.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SupervisionOp {
    LinkSupervisor,
    UnlinkSupervisor,
    RequireCosign,
    LiftCosign,
    GrantAssigner,
    WithdrawAssigner,
}

impl SupervisionOp {
    /// The wire word the SQL doors take.
    pub fn as_sql(self) -> &'static str {
        match self {
            Self::LinkSupervisor => "link-supervisor",
            Self::UnlinkSupervisor => "unlink-supervisor",
            Self::RequireCosign => "require-cosign",
            Self::LiftCosign => "lift-cosign",
            Self::GrantAssigner => "grant-assigner",
            Self::WithdrawAssigner => "withdraw-assigner",
        }
    }

    /// Exactly the six words; anything else (case included) is `None`.
    pub fn from_cli(word: &str) -> Option<Self> {
        [
            Self::LinkSupervisor,
            Self::UnlinkSupervisor,
            Self::RequireCosign,
            Self::LiftCosign,
            Self::GrantAssigner,
            Self::WithdrawAssigner,
        ]
        .into_iter()
        .find(|op| op.as_sql() == word)
    }

    /// Does this act name a second person (the supervisor)?
    pub fn needs_supervisor(self) -> bool {
        match self {
            Self::LinkSupervisor | Self::UnlinkSupervisor => true,
            Self::RequireCosign
            | Self::LiftCosign
            | Self::GrantAssigner
            | Self::WithdrawAssigner => false,
        }
    }
}

/// The supervisor argument is required for exactly the two link ops. Pure, so the CLI (and the
/// tests) share one rule; the doors enforce it again in SQL.
pub fn check_supervisor_arg(op: SupervisionOp, supervisor: Option<&str>) -> Result<(), String> {
    match (op.needs_supervisor(), supervisor) {
        (true, None) => Err(format!("{} needs --supervisor", op.as_sql())),
        (false, Some(_)) => Err(format!("{} takes no --supervisor", op.as_sql())),
        _ => Ok(()),
    }
}

/// One supervision act approved by `approver_staff_id`, signed by `approver`. The token is bound
/// to the family's current state (`cairn_supervision_prior`), so a stale token cannot replay.
pub async fn act(
    db: &Client,
    op: SupervisionOp,
    subject: &str,
    supervisor: Option<&str>,
    approver_staff_id: &str,
    approver: &dyn EventSigner,
) -> anyhow::Result<()> {
    let op = op.as_sql();
    let prior: i64 = db
        .query_one(
            "SELECT cairn_supervision_prior($1, $2, $3)",
            &[&op, &subject, &supervisor],
        )
        .await
        .context("reading the supervision state")?
        .get(0);
    let addr: Vec<u8> = db
        .query_one(
            "SELECT cairn_supervision_address($1, $2, $3, $4)",
            &[&op, &subject, &supervisor, &prior],
        )
        .await
        .context("computing the supervision address")?
        .get(0);
    let token = approve(&addr, approver)?;
    db.execute(
        "SELECT cairn_supervision_act($1, $2, $3, $4, $5)",
        &[&op, &subject, &supervisor, &approver_staff_id, &token],
    )
    .await
    .map_err(|e| LocalDbFault::new("supervision act", e))?;
    Ok(())
}

/// Console-only (owner connection): the same acts with nobody to approve them.
pub async fn console_act(
    db: &Client,
    op: SupervisionOp,
    subject: &str,
    supervisor: Option<&str>,
) -> anyhow::Result<()> {
    db.execute(
        "SELECT cairn_supervision_console_act($1, $2, $3)",
        &[&op.as_sql(), &subject, &supervisor],
    )
    .await
    .map_err(|e| LocalDbFault::new("console supervision act", e))?;
    Ok(())
}

/// One member in the supervision panel.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StaffSupervision {
    pub staff_id: String,
    pub display_name: String,
    pub role: String,
    pub needs_cosigning: bool,
    pub assigner: bool,
    /// Staff IDs of the member's current supervisors.
    pub supervisors: Vec<String>,
}

/// Every member with their supervision state now (`cairn_supervision_staff`).
pub async fn staff(db: &Client) -> anyhow::Result<Vec<StaffSupervision>> {
    let rows = db
        .query(
            "SELECT staff_id, display_name, role, needs_cosigning, assigner, supervisors \
               FROM cairn_supervision_staff()",
            &[],
        )
        .await
        .context("reading the supervision panel")?;
    Ok(rows
        .iter()
        .map(|r| StaffSupervision {
            staff_id: r.get(0),
            display_name: r.get(1),
            role: r.get(2),
            needs_cosigning: r.get(3),
            assigner: r.get(4),
            supervisors: r.get(5),
        })
        .collect())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn supervisor_arg_is_required_for_exactly_the_link_ops() {
        use SupervisionOp::*;
        for op in [LinkSupervisor, UnlinkSupervisor] {
            assert!(check_supervisor_arg(op, Some("s2")).is_ok());
            assert!(check_supervisor_arg(op, None).is_err());
        }
        for op in [RequireCosign, LiftCosign, GrantAssigner, WithdrawAssigner] {
            assert!(check_supervisor_arg(op, None).is_ok());
            assert!(check_supervisor_arg(op, Some("s2")).is_err());
        }
    }
}
