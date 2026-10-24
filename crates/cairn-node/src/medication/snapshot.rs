//! #864 — a medication list is read from ONE snapshot.
//!
//! `read::list_patient_medications` assembles a list from up to fourteen statements (see that
//! module's header for why). Under READ COMMITTED each statement sees a fresh snapshot, so a
//! stop committed by sync or a colleague between the cessation-reasons read and the list views
//! gave a line that read **stopped** with **no reason** — for a reason that may be the only
//! record of an adverse reaction. `in_snapshot` runs the whole read inside one
//! `REPEATABLE READ READ ONLY` transaction: every statement sees the same committed state.
//!
//! Only for READS on a connection not already in a transaction. Never raise the isolation of a
//! transaction that writes through a door: db/005's `cairn_lock_custody` note explains why a
//! REPEATABLE READ door caller reopens the custody/shred race. Read-only also makes Postgres
//! refuse any write slipped inside.
use tokio_postgres::{Client, IsolationLevel, Transaction};

/// Run `read` against a `REPEATABLE READ READ ONLY` transaction opened on `client`, then end it.
///
/// The production list readers (the CLI's `medication-list` and `medication-cease` notes, the
/// window's chart read, the sign-off's two reads) go through here. Tests may still call the
/// readers on a bare client or an aborted transaction (their failure seam).
pub async fn in_snapshot<T>(
    client: &mut Client,
    read: impl AsyncFnOnce(&Transaction<'_>) -> anyhow::Result<T>,
) -> anyhow::Result<T> {
    let tx = client
        .build_transaction()
        .isolation_level(IsolationLevel::RepeatableRead)
        .read_only(true)
        .start()
        .await?;
    let out = read(&tx).await?;
    // Nothing was written; COMMIT just ends the transaction (an Err path drops `tx`, which
    // rolls back the same nothing).
    tx.commit().await?;
    Ok(out)
}
