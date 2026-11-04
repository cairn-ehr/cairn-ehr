//! Who's here — the staff a terminal may name as the person writing (sign-in slice 3b-1).
//!
//! "Who's here" is a CLAIM, never authentication (design §3.1): naming someone signs nothing and
//! releases no key. It only says whose words a draft part is. So the picker lists the members
//! whose role may act clinically — never a superuser or an admin, who confer no clinical
//! authorship (§5.1) — through db/066's `cairn_whos_here_directory` door, because `vault_member`
//! itself has no grant to anyone.
use tokio_postgres::GenericClient;

/// One person the picker can offer.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StaffEntry {
    /// The normalised vault staff id: what a draft part records as its writer.
    pub staff_id: String,
    pub display_name: String,
    pub role: String,
}

/// The clinical members of this node's vault, by display name.
pub async fn directory(client: &(impl GenericClient + Sync)) -> anyhow::Result<Vec<StaffEntry>> {
    let rows = client
        .query(
            "SELECT staff_id, display_name, role FROM cairn_whos_here_directory()",
            &[],
        )
        .await?;
    Ok(rows
        .iter()
        .map(|r| StaffEntry {
            staff_id: r.get(0),
            display_name: r.get(1),
            role: r.get(2),
        })
        .collect())
}
