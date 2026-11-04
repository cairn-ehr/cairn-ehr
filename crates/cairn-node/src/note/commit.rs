//! The quarantine commit (sign-in slice 3b-2a): a draft nobody has typed into for the site's
//! quarantine time goes into the record by itself, UNSIGNED, as node-signed
//! `clinical.note.asserted` events — one per non-blank part, chained by `addendum-to` exactly as
//! a Sign chains them (`sign.rs`).
//!
//! WHY. On paper an entry left unsigned on the progress sheet is simply in the record: anyone
//! can read it, and it plainly carries no signature. A Cairn draft is not in the record until
//! someone signs it, so an abandoned one would be invisible to the next clinician. The timer
//! closes that gap without pretending anyone signed: the NODE signs (it only proves "this node
//! recorded these words"), and a named writer is a `drafted` contributor, which the read grades
//! `Asserted` ("went in unsigned"), never `Attested`.
//!
//! WHO DECIDES "OVERDUE". Never this code and never a terminal clock: db/067's commit door
//! re-decides on the database's clock, under the draft's advisory lock, in the same call that
//! consumes the draft. [`overdue_drafts`] is only a hint of which drafts to try. So two windows
//! ticking at once, or a Sign racing the timer, put exactly one set of notes in the log: the
//! loser's door call answers zero rows ([`CommitOutcome::NotDue`]).
//!
//! THE ONE-TRANSACTION SHAPE is `sign_note`'s: the door spends the draft, each part is built,
//! sealed and submitted with NO savepoint (db/065's commit-time trigger checks the note row's
//! xmin is this transaction's), and either all of it commits or none of it does.
//!
//! A NAMED WRITER WITH NO VAULT KEY makes the draft STUCK: the transaction is dropped, so the
//! draft stays exactly as it was (and keeps showing overdue). It is never committed as "writer
//! not known": that would quietly lose who wrote it (principle 4, as `SignRefusal::NoSuchWriter`).
use cairn_event::contributor::with_drafting_writer;
use cairn_event::SigningKey;
use chrono::{DateTime, Duration, Utc};
use tokio_postgres::GenericClient;
use uuid::Uuid;

use super::draft::commit_overdue_parts;
use super::sign::{body_from_spent, writer_key};
use crate::db_diagnosis::Committed;
use crate::medication::sealed_submit::{prepare_sealed_as, Prepared};

/// When a draft last typed into at `last_keystroke` falls due, under a quarantine of
/// `quarantine_minutes` (`cairn_note_quarantine_minutes()`). Pure; the window words it, the
/// database alone decides it.
pub fn draft_due(last_keystroke: DateTime<Utc>, quarantine_minutes: i32) -> DateTime<Utc> {
    last_keystroke + Duration::minutes(i64::from(quarantine_minutes))
}

/// What one draft's commit did.
#[derive(Debug)]
pub enum CommitOutcome {
    /// The draft went in: its parts' events in writing order (`event_id` = the first, the note).
    Committed(Committed<Vec<Uuid>>),
    /// Nothing to write: the draft is absent, already spent, not yet overdue, or was all blank
    /// (the door deleted it, and that deletion was committed).
    NotDue,
    /// A named writer has no vault key any more: rolled back, the draft is kept as it was.
    Stuck,
}

/// A part's named writer whose member row has gone (see the module doc).
#[derive(Debug, PartialEq, Eq)]
struct Stuck;

/// The `drafted` key for a part: `None` for a writer not known (the part is the node's alone),
/// the writer's current key for a named writer, and [`Stuck`] for a named writer with no key.
/// Pure.
fn part_writer_kid(writer: Option<&str>, kid: Option<String>) -> Result<Option<String>, Stuck> {
    match (writer, kid) {
        (None, _) => Ok(None),
        (Some(_), None) => Err(Stuck),
        (Some(_), Some(kid)) => Ok(Some(kid)),
    }
}

/// The drafts overdue now, on the database's clock, on `patient`'s chart (`None` = every chart),
/// oldest first. A HINT only: [`commit_overdue`] re-decides under the draft's lock.
pub async fn overdue_drafts(
    client: &(impl GenericClient + Sync),
    patient: Option<Uuid>,
) -> anyhow::Result<Vec<Uuid>> {
    let rows = client
        .query(
            "SELECT draft_id::text FROM cairn_note_drafts_overdue($1::text::uuid)",
            &[&patient.map(|p| p.to_string())],
        )
        .await?;
    rows.iter()
        .map(|r| Ok(r.get::<_, String>(0).parse()?))
        .collect()
}

/// Commit draft `draft` if it is overdue, as node-signed note events (see the module doc), in
/// one transaction. `node_origin` is the HLC's origin (`identity::load_local`'s `node_id_hex`).
/// `Err` is a database failure or a floor refusal, which wrote nothing — unless the commit's
/// answer was lost, which `commit_failure` marks.
pub async fn commit_overdue(
    client: &mut tokio_postgres::Client,
    node_sk: &SigningKey,
    draft: Uuid,
    node_origin: &str,
) -> anyhow::Result<CommitOutcome> {
    let first = Uuid::now_v7();
    let node_kid = hex::encode(node_sk.verifying_key().to_bytes());
    // Every early return below drops `tx` uncommitted, which rolls the whole commit back.
    let tx = client.transaction().await?;
    let parts = commit_overdue_parts(&tx, draft, first).await?;
    if parts.is_empty() {
        // Commit even so: an overdue all-blank draft was deleted by the door, and that must
        // stick. Nothing else was written, and no event names this commit.
        tx.commit().await?;
        return Ok(CommitOutcome::NotDue);
    }
    let mut ids: Vec<Uuid> = Vec::with_capacity(parts.len());
    for part in &parts {
        let kid = writer_key(&tx, part.writer.as_deref()).await?;
        let Ok(writer_kid) = part_writer_kid(part.writer.as_deref(), kid) else {
            return Ok(CommitOutcome::Stuck);
        };
        let id = if ids.is_empty() {
            first
        } else {
            Uuid::now_v7()
        };
        // One tick per part, INSIDE the transaction, as a Sign's.
        let hlc = crate::db::next_hlc(&tx, node_origin).await?;
        let body = body_from_spent(id, part, &node_kid, hlc, ids.last().copied());
        let body = with_drafting_writer(body, writer_kid.as_deref());
        let Prepared {
            signed_bytes, dek, ..
        } = prepare_sealed_as(&tx, body, node_sk).await?;
        // NO savepoint, for `sign_note`'s reason: db/065's commit-time trigger needs the note
        // row's xmin to be this transaction's own id.
        tx.execute(
            "SELECT submit_event($1, NULL, NULL, $2)",
            &[&signed_bytes, &dek.as_bytes().as_slice()],
        )
        .await?;
        ids.push(id);
    }
    tx.commit()
        .await
        .map_err(|e| crate::db_diagnosis::commit_failure(first, e.into()))?;
    Ok(CommitOutcome::Committed(Committed {
        value: ids,
        event_id: first,
    }))
}

/// What one pass of the timer did: how many drafts went in, which were stuck, and which failed
/// (with the error in words). The caller reports it; nothing here prints.
#[derive(Debug, Default)]
pub struct CommitReport {
    pub committed: usize,
    pub stuck: Vec<Uuid>,
    pub failed: Vec<(Uuid, String)>,
}

/// Commit every overdue draft on `patient`'s chart (`None` = every chart), each in its own
/// transaction. One draft's failure is recorded and the pass goes on, so one bad draft never
/// holds the others back (Review Focus 4). Only the listing's own failure is `Err`.
pub async fn commit_overdue_drafts(
    client: &mut tokio_postgres::Client,
    node_sk: &SigningKey,
    patient: Option<Uuid>,
    node_origin: &str,
) -> anyhow::Result<CommitReport> {
    let mut report = CommitReport::default();
    for draft in overdue_drafts(client, patient).await? {
        match commit_overdue(client, node_sk, draft, node_origin).await {
            Ok(CommitOutcome::Committed(_)) => report.committed += 1,
            Ok(CommitOutcome::NotDue) => {}
            Ok(CommitOutcome::Stuck) => report.stuck.push(draft),
            Err(e) => report.failed.push((draft, format!("{e:#}"))),
        }
    }
    Ok(report)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn draft_due_adds_the_minutes() {
        let at: DateTime<Utc> = "2026-11-04T01:00:00.123456Z".parse().unwrap();
        assert_eq!(
            draft_due(at, 180),
            "2026-11-04T04:00:00.123456Z"
                .parse::<DateTime<Utc>>()
                .unwrap()
        );
        assert_eq!(
            draft_due(at, 43200),
            "2026-12-04T01:00:00.123456Z"
                .parse::<DateTime<Utc>>()
                .unwrap()
        );
    }

    /// A named writer with no key is stuck, never "not known"; a writer not known is the node's
    /// part alone, whatever key is passed.
    #[test]
    fn a_named_writer_without_a_key_is_stuck() {
        assert_eq!(part_writer_kid(Some("gone"), None), Err(Stuck));
        assert_eq!(
            part_writer_kid(Some("nurse-a"), Some("K".into())),
            Ok(Some("K".into()))
        );
        assert_eq!(part_writer_kid(None, None), Ok(None));
    }
}
