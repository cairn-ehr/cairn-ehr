//! A chart's notes, as the window lists them.
//!
//! Linked charts read as one, as the medication list does (ADR-0076): the notes of every chart
//! `person_charts` says is this person. One exception, the same as the medication list's
//! (#697 (b)): while the set holds a link this node DOUBTS, a note on a chart other than the
//! opened one may be a stranger's, so it is counted (`withheld_on_linked`) and left out.
//!
//! Notes are read ONLY from the `clinical_note` projection, never from the event log's clear
//! view (ruling R8): the projection is what keeps a newer, unchecked note profile carried by a
//! peer off the screen. `event_log` is joined only for what grades authorship.
//!
//! Discarded drafts (slice 3b-2b) are listed BESIDE the notes, as markers, read only from the
//! `clinical_note_discard` projection and under the same doubted-link rule. They are never
//! joined into the notes, never counted as one, and `NOTES_SQL` does not know they exist.
use anyhow::Context;
use cairn_event::contributor::{classify_authorship_with_humans, VerifiedKid};
use cairn_event::note::unescape_note_markup;
use cairn_event::note_discard::DiscardedWriter;
use chrono::{DateTime, Utc};
use tokio_postgres::{GenericClient, Row};
use uuid::Uuid;

use super::draft::{open_drafts, NoteDraft};
use super::supervision::Supervision;
use super::view::{note_grade, part_writer, NoteGrade, PartWriter};
use super::vouch_read::{parse_vouches, signer_of, vouches_column, PartVouch, Signer};

/// One signed note, ready for the window to word (`view::time_words`, `view::grade_words`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NoteRow {
    pub event_id: Uuid,
    /// The chart the note was written on (a linked chart's, when the read combines them).
    pub chart: Uuid,
    /// The text as the writer typed it (the signed narrative, unescaped).
    pub text: String,
    pub t_effective: DateTime<Utc>,
    /// `"stated"` or `"recording-started"`.
    pub effective_basis: String,
    pub recording_started: DateTime<Utc>,
    /// When the part entered the record: its `event_log.hlc_wall`, never `t_effective`. The
    /// instant the supervision record is judged "as of" (`supervision`).
    pub recorded_at: DateTime<Utc>,
    /// The supervision RECORD said the writer needed co-signing at `recorded_at` (a student's
    /// role is not folded in here; `cosign` combines them). Default false.
    pub needs_cosigning: bool,
    /// The writer's supervisors NOW (actors, lower hex). Default empty.
    pub writer_supervisors: Vec<String>,
    pub grade: NoteGrade,
    /// The note this one continues (its `addendum-to` ref): a later writer's part of one draft,
    /// or an addendum to a note already in the record (slice 3c-1, ADR-0091).
    pub addendum_to: Option<Uuid>,
    /// Who typed it (`view::part_writer`): the signer, a `drafted` contributor (named when this
    /// node can name that key, through `cairn_note_signer_name`), or nobody recorded when the
    /// signer only attested it.
    pub writer: PartWriter,
    /// Who the `Signed` grade authenticated (slice 3c-2): `Some` exactly when `grade` is
    /// `NoteGrade::Signed`.
    pub signed_by: Option<Signer>,
    /// The actor of the part's `drafted` contributor key (lower hex), when this node knows it.
    pub drafted_actor: Option<String>,
    /// The actor of the key that signed the part's ENVELOPE (`signer_key_id`), only when the
    /// registry enrolled that key for a human (`cairn_note_key_is_human`) and can place it; `None`
    /// otherwise. It is what db/071's co-sign rule reads (not the grade's key, which may be an
    /// attester), so `vouch::sign_off_vouches` plans `CoSigned` only where the floor admits it.
    pub signer_human_actor: Option<String>,
    /// The part's verified vouches for its own bytes, in HLC order (`vouch_read`).
    pub vouches: Vec<PartVouch>,
}

/// One discarded draft, as the chart lists it beside (never among) its notes.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DiscardMarker {
    /// The discard event this marker was projected from.
    pub event_id: Uuid,
    /// The chart the draft was on (a linked chart's, when the read combines them).
    pub chart: Uuid,
    /// The discard instant (the event's `t_effective`).
    pub discarded_at: DateTime<Utc>,
    /// Who discarded it (`cairn_note_signer_name` of the signing key); `None` if not nameable.
    pub discarder: Option<String>,
    /// Its non-blank parts, in writing order.
    pub parts: Vec<DiscardedText>,
}

/// One part of a discarded draft.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DiscardedText {
    /// The discarder wrote it; `false` = writer not known.
    pub by_discarder: bool,
    /// The text as typed (unescaped).
    pub text: String,
    /// The writer's claimed time for this part; `None` means not stated (principle 4).
    pub stated: Option<DateTime<Utc>>,
    /// When the part began being recorded.
    pub recording_started: DateTime<Utc>,
    /// The part's last real keystroke.
    pub recording_ended: DateTime<Utc>,
}

/// What the note pane shows for one opened chart.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PatientNoteList {
    /// Newest `t_effective` first.
    pub notes: Vec<NoteRow>,
    /// Notes on other charts of a set holding a doubted link: counted, not shown.
    pub withheld_on_linked: usize,
    /// The opened chart's open drafts.
    pub drafts: Vec<NoteDraft>,
    /// Discarded drafts on the set's charts, newest `discarded_at` first (slice 3b-2b).
    pub discards: Vec<DiscardMarker>,
    /// Discard markers on other charts of a set holding a doubted link: counted, not shown.
    pub discards_withheld_on_linked: usize,
}

/// The notes of every chart in `charts`, with what grades their authorship.
///
/// The attester comes through db/065's `cairn_note_vouched_attester` door, which answers only
/// for an attester `cairn_attestation_vouched` clears: db/020 can store a peer's attester key
/// UNVERIFIED, and minting a `VerifiedKid` from such a row would re-open #412 (see
/// `VerifiedKid::from_event_log_column`). A door, because the window role may not call the
/// predicate itself (ruling R11). `signer_key_id` needs no such check — no row exists unless
/// the door verified the signature against it.
///
/// The addendum ref is read as `::uuid::text`, Postgres's canonical form: the floor admits any
/// spelling `::uuid` accepts, some of which Rust's `Uuid` parser refuses, and one such ref must not
/// make the chart's whole note list fail to read.
///
/// The two columns before the last say who typed the part (`view::part_writer`): whether a `drafted`
/// contributor is present even when its key cannot be named, and whether the signer only
/// ATTESTED it — so a part someone else typed never reads like the signer's own note (I2).
///
/// The last column is the `drafted` contributor's key itself (slice 3b-2a), and only when db/067's
/// `cairn_note_key_is_human` says the registry enrolled it for a HUMAN actor. The grader is told
/// that key is a person, so a quarantine-committed part grades `Asserted` ("went in unsigned")
/// rather than `Device`. Being NAMED is not enough: `cairn_note_signer_name` also names device and
/// agent keys by their pinned handle (final review M6). It is never an attester: the #412 guard
/// below still holds.
///
/// `g.k` is the key the `Signed` grade authenticates: the verified attester when there is one,
/// else the signer (db/005 stores a token only for the contributor that bears responsibility, so
/// the attester, when present, is whom the grade names); the signer name column is read for it
/// too, so `signed_by` and the grade name the same person. The three columns after the human
/// drafted key say who `g.k` is (`vouch_read::signer_of`); the next is the drafted key's actor
/// (for `vouch::addendum_covers`). Then come the part's verified vouches
/// (`vouch_read::vouches_column!`), read in this same statement so they share its snapshot. The
/// last column is the ENVELOPE signer's actor, only for a human's key: what db/071's co-sign rule
/// reads (`NoteRow::signer_human_actor`). The very last column is the part's `e.hlc_wall`
/// (ms since the epoch), what `NoteRow::recorded_at` and the supervision record are judged at.
///
/// **Four CTEs, for cost, not meaning (#1021).** `base` reads each part's KEYS and time once (the
/// WHERE runs here, and the attester door once per part); `facts` asks the key doors about each
/// DISTINCT key those parts name (`k`, the drafted key, the envelope signer) exactly once; `kept`
/// is the parts the second argument KEEPS; `rows` builds everything else that is a fact of the part
/// (text, contributors, vouches) for those parts only. The final SELECT joins `rows` and `facts`, so the columns
/// above are unchanged. Measured: per-row key lookups were 88% of a 100 000-note read over 40
/// keys; resolved once per key the same read was 5–6× faster with identical output. A NULL key (no
/// drafted contributor) is left out of `facts`, so its columns read NULL, exactly what the doors
/// answer for NULL; `k` and the signer are never NULL. One statement, one snapshot: a key's facts
/// cannot differ between two rows that name it.
///
/// The keep rule sees `b` (the part's `base` row) and `fk` (the facts of its `k`, LEFT-joined). The
/// chart list keeps everything (`TRUE`); the menu keeps only what it could list (`NOTES_BY_KEYS_SQL`).
macro_rules! notes_sql {
    ($where:literal, $keep:literal) => {
        concat!(
    // `base`: one row per part, its keys and time only (the WHERE runs here).
    "WITH base AS MATERIALIZED (SELECT n.event_id, e.signer_key_id, g.attester, \
            (SELECT c ->> 'actor_id' FROM jsonb_array_elements(e.contributors) c \
              WHERE c ->> 'role' = 'drafted' LIMIT 1) AS drafted, e.hlc_wall \
       FROM clinical_note n JOIN event_log e ON e.event_id = n.event_id \
      CROSS JOIN LATERAL (SELECT cairn_note_vouched_attester(e.event_id) AS attester) g \
      WHERE ", $where, "), \
     facts AS MATERIALIZED (SELECT key, cairn_note_signer_name(key) AS name, cairn_note_key_actor(key) AS actor, \
            cairn_note_key_role(key) AS role, cairn_key_confirmation(key) AS confirmation, \
            cairn_note_key_is_human(key) AS human \
       FROM (SELECT COALESCE(attester, signer_key_id) FROM base UNION SELECT drafted FROM base \
             UNION SELECT signer_key_id FROM base) u(key) \
      WHERE key IS NOT NULL), \
     kept AS MATERIALIZED (SELECT b.* FROM base b \
       LEFT JOIN facts fk ON fk.key = COALESCE(b.attester, b.signer_key_id) \
      WHERE ", $keep, "), \
     rows AS MATERIALIZED (SELECT n.event_id::text AS event_id, n.patient_id::text AS patient_id, \
            n.narrative, n.t_effective, n.effective_basis, n.recording_started, b.signer_key_id, \
            e.contributors::text AS contributors, b.attester, \
            COALESCE(b.attester, b.signer_key_id) AS k, \
            (SELECT (r ->> 'event')::uuid::text FROM jsonb_array_elements(n.refs) r \
              WHERE r ->> 'rel' = 'addendum-to' LIMIT 1) AS addendum_to, \
            EXISTS (SELECT 1 FROM jsonb_array_elements(e.contributors) c WHERE c ->> 'role' = 'drafted') \
              AS has_drafted, \
            EXISTS (SELECT 1 FROM jsonb_array_elements(e.contributors) c \
                     WHERE c ->> 'role' = 'attested' AND c ->> 'actor_id' = e.signer_key_id) AS signer_attested, \
            b.drafted, ",
    vouches_column!(),
    " AS vouches, b.hlc_wall \
       FROM kept b JOIN clinical_note n ON n.event_id = b.event_id JOIN event_log e ON e.event_id = b.event_id) \
    SELECT r.event_id, r.patient_id, r.narrative, r.t_effective, r.effective_basis, r.recording_started, \
           r.signer_key_id, r.contributors, r.attester, fk.name, r.addendum_to, fd.name, r.has_drafted, \
           r.signer_attested, CASE WHEN fd.human THEN r.drafted END, \
           fk.actor, fk.role, fk.confirmation, fd.actor, r.vouches, \
           CASE WHEN fs.human THEN fs.actor END, r.hlc_wall \
      FROM rows r \
      LEFT JOIN facts fk ON fk.key = r.k \
      LEFT JOIN facts fd ON fd.key = r.drafted \
      LEFT JOIN facts fs ON fs.key = r.signer_key_id")
    };
}

/// The chart list's query: the parts on the given charts (`$1`, text uuids).
const NOTES_SQL: &str = notes_sql!("n.patient_id = ANY($1::text[]::uuid[])", "TRUE");

/// The same columns and joins, for the parts the given KEYS (`$1`, text) wrote: signed with one
/// of them, or carrying one as a contributor (a part that went in unsigned names its drafter only
/// there). Only the WHERE and the keep rule differ, so the menu's rows cannot drift from the chart's.
///
/// **The keep rule (#1021)** drops, before the costly columns are built, every part that no
/// person's menu could list (`note::menu::classify`). It must stay a SUPERSET: a part it drops
/// silently leaves someone's "Waiting for you". `$2` is the review window's start (ms, the menu's
/// own `now` minus `REVIEW_DAYS`), `$3` every actor the record ever required co-signing for
/// (`Supervision::ever_required_cosign`). A part is kept when ANY of these holds:
///
/// - `b.drafted IS NOT NULL`: it names a drafted writer. Group 1's "went in unsigned" needs one,
///   and so does every co-sign rule for a part not signed with its own envelope
///   (`cosign::writer_signature` falls back to the drafted writer's vouch).
/// - `b.hlc_wall >= $2`: within the review window, so group 3 may list it.
/// - `fk.confirmation = 'repudiated'`: its signature's key is repudiated (group 1's "key
///   repudiated", `vouch::counts_as_unsigned`; `signer_of` reads the same word).
/// - `fk.role = 'student'` or `fk.actor = ANY($3)`: its signer signed as a student, or was ever
///   required to be co-signed (group 2, `cosign::cosign_required`). The record is judged at the
///   part's time in Rust; "ever" is the superset.
///
/// Anything else names no drafted writer, is signed under a key not repudiated by a non-student
/// the record never required co-signing for, and is outside the window: owed to no one, listed by
/// no one. `tests/note_person_menu.rs` re-reads every scenario 30 days on, when only the other
/// terms can keep a part, and asserts the same groups 1 and 2; removing any one term fails a test.
// shortcut: `base` still scans every part the keys ever wrote, one attester-door call each, and
// every part by a student or a writer ever required co-signing is kept, cleared or not. Measured
// by `examples/measure_person_menu.rs` (results/2026-11-08-person-menu-reads-macos-m3max.md).
// Upgrade to a projection of open obligations when a node outgrows it, see #1021.
const NOTES_BY_KEYS_SQL: &str = notes_sql!(
    "e.signer_key_id = ANY($1::text[]) OR EXISTS (SELECT 1 FROM jsonb_array_elements(e.contributors) c \
      WHERE c ->> 'actor_id' = ANY($1::text[]))",
    "b.drafted IS NOT NULL OR b.hlc_wall >= $2::bigint OR fk.confirmation = 'repudiated' \
      OR fk.role = 'student' OR fk.actor = ANY($3::text[])"
);

/// One row per PART of each discard on the given charts, with its 1-based position (`ord`).
///
/// Only the five named part keys are read: db/068 stores `parts` as received (its floor does not
/// refuse an extra key), so a key a raw-SQL client added beside them must never reach the screen.
/// The times go through `cairn_t_effective`, the very validator the floor admitted them with, so
/// every row the floor let in reads here: as `NOTES_SQL` reads a ref as `::uuid::text`, one odd
/// spelling cannot make the chart's whole list fail. `stated_time` JSON null reads as SQL NULL (not stated).
const DISCARDS_SQL: &str = "\
    SELECT d.event_id::text, d.patient_id::text, d.t_effective, \
           cairn_note_signer_name(e.signer_key_id), x.ord, \
           x.p ->> 'writer', x.p ->> 'narrative', cairn_t_effective(x.p ->> 'stated_time'), \
           cairn_t_effective(x.p ->> 'recording_started'), \
           cairn_t_effective(x.p ->> 'recording_ended') \
      FROM clinical_note_discard d JOIN event_log e ON e.event_id = d.event_id \
     CROSS JOIN LATERAL jsonb_array_elements(d.parts) WITH ORDINALITY AS x(p, ord) \
     WHERE d.patient_id = ANY($1::text[]::uuid[])";

/// Whether a stored discard part's `writer` word names the discarder. The floor admits only the
/// two words this build knows, so an unknown word (or none) is an ERROR, never "not known": a
/// later value must not be shown as either (PR #984 review). The match is exhaustive, so a new
/// `DiscardedWriter` value cannot compile without deciding how the marker heads it. Unlike an odd
/// time spelling (above), such a word fails the chart's list: the floor admitted no such row, so
/// only a defect can put one there, and saying so beats heading someone's text with the wrong
/// writer.
fn by_discarder(word: Option<&str>) -> anyhow::Result<bool> {
    match word.and_then(DiscardedWriter::from_wire) {
        Some(DiscardedWriter::Discarder) => Ok(true),
        Some(DiscardedWriter::NotKnown) => Ok(false),
        None => {
            anyhow::bail!("a discarded part's writer is {word:?}, which this build does not know")
        }
    }
}

/// The discard markers of `members`, minus those a doubted set withholds (on a chart other than
/// `patient`), which are only counted. Returns `(markers newest first, withheld count)`.
async fn discard_markers(
    client: &(impl GenericClient + Sync),
    patient: Uuid,
    members: &[String],
    doubted: bool,
) -> anyhow::Result<(Vec<DiscardMarker>, usize)> {
    // (ord, marker-with-this-part) per row; grouped below by event. Ordered in Rust (ADR-0045).
    let mut rows: Vec<(i64, DiscardMarker)> = Vec::new();
    for r in client.query(DISCARDS_SQL, &[&members]).await? {
        rows.push((
            r.get(4),
            DiscardMarker {
                event_id: r.get::<_, String>(0).parse()?,
                chart: r.get::<_, String>(1).parse()?,
                discarded_at: r.get(2),
                discarder: r.get(3),
                parts: vec![DiscardedText {
                    by_discarder: by_discarder(r.get::<_, Option<String>>(5).as_deref())?,
                    text: unescape_note_markup(&r.get::<_, String>(6)),
                    stated: r.get(7),
                    recording_started: r.get(8),
                    recording_ended: r.get(9),
                }],
            },
        ));
    }
    rows.sort_by_key(|(ord, m)| (m.event_id, *ord));
    let mut markers: Vec<DiscardMarker> = Vec::new();
    for (_, m) in rows {
        match markers.last_mut() {
            Some(last) if last.event_id == m.event_id => last.parts.extend(m.parts),
            _ => markers.push(m),
        }
    }
    let before = markers.len();
    markers.retain(|m| !doubted || m.chart == patient);
    let withheld = before - markers.len();
    markers.sort_by_key(|m| std::cmp::Reverse((m.discarded_at, m.event_id)));
    Ok((markers, withheld))
}

/// One `NOTES_SQL` / `NOTES_BY_KEYS_SQL` row as a `NoteRow`, annotated against the supervision
/// record. The single parser of that column list: the chart list and the key read both call it.
fn note_row(r: &Row, sup: &Supervision) -> anyhow::Result<NoteRow> {
    let chart: Uuid = r.get::<_, String>(1).parse()?;
    let signer: String = r.get(6);
    let contributors: serde_json::Value = serde_json::from_str(&r.get::<_, String>(7))?;
    let attester: Option<String> = r.get(8);
    let drafted_by: Option<String> = r.get(11);
    // The drafted key, present only when the registry knows it as a human's (see NOTES_SQL).
    let human_drafted_kid: Option<String> = r.get(14);
    let humans: Vec<&str> = human_drafted_kid.as_deref().into_iter().collect();
    // Both kids come from event_log columns whose existence is a verification: the signer
    // always, the attester only through the vouch door in NOTES_SQL.
    let confidence = classify_authorship_with_humans(
        &contributors,
        VerifiedKid::from_event_log_column(&signer),
        attester.as_deref().map(VerifiedKid::from_event_log_column),
        &humans,
    );
    let grade = note_grade(confidence, r.get(9));
    let signed_by = matches!(grade, NoteGrade::Signed { .. })
        .then(|| {
            signer_of(
                r.get(15),
                r.get(9),
                r.get::<_, Option<String>>(16).as_deref(),
                r.get::<_, Option<String>>(17).as_deref(),
            )
        })
        .transpose()?;
    let recorded_at = DateTime::from_timestamp_millis(r.get::<_, i64>(21))
        .context("a note part's hlc_wall is out of range")?;
    let mut part = NoteRow {
        event_id: r.get::<_, String>(0).parse()?,
        chart,
        text: unescape_note_markup(&r.get::<_, String>(2)),
        t_effective: r.get(3),
        effective_basis: r.get(4),
        recording_started: r.get(5),
        recorded_at,
        needs_cosigning: false,
        writer_supervisors: vec![],
        grade,
        addendum_to: r
            .get::<_, Option<String>>(10)
            .map(|s| s.parse())
            .transpose()?,
        writer: part_writer(r.get(12), drafted_by, r.get(13)),
        signed_by,
        drafted_actor: r.get(18),
        signer_human_actor: r.get(20),
        vouches: parse_vouches(&r.get::<_, String>(19))?,
    };
    super::supervision::annotate(&mut part, sup);
    Ok(part)
}

/// Every note part the given keys wrote that some menu could list (see `NOTES_BY_KEYS_SQL`'s keep
/// rule; `review_from` is the review window's start), each annotated against `sup`. Unordered: the
/// caller orders.
///
/// The doubted-link withholding of `list_patient_notes` is NOT applied: that rule guards the
/// combining of linked charts on one opened chart, and the menu opens each part's own chart.
pub async fn parts_by_keys(
    client: &(impl GenericClient + Sync),
    keys: &[String],
    sup: &Supervision,
    review_from: DateTime<Utc>,
) -> anyhow::Result<Vec<NoteRow>> {
    let review_from_ms = review_from.timestamp_millis();
    let cosign_writers = sup.ever_required_cosign();
    client
        .query(
            NOTES_BY_KEYS_SQL,
            &[&keys, &review_from_ms, &cosign_writers],
        )
        .await?
        .iter()
        .map(|r| note_row(r, sup))
        .collect()
}

/// Read the notes of the person `patient` is a chart of, plus `patient`'s open drafts and the
/// set's discard markers.
pub async fn list_patient_notes(
    client: &(impl GenericClient + Sync),
    patient: Uuid,
) -> anyhow::Result<PatientNoteList> {
    let charts = crate::patient::person::person_charts(client, patient).await?;
    let doubted = crate::medication::read::set_has_doubted_link(client, &charts).await?;
    let members: Vec<String> = charts.members().iter().map(Uuid::to_string).collect();
    // The supervision record is read ONCE, on this client, then every part is judged against it
    // in pure code (`supervision::annotate`). It shares the notes' snapshot only when the caller
    // gives one (the window's `in_snapshot`, REPEATABLE READ); under READ COMMITTED (as in
    // `vouch::vouch_as_shown`) the record and the notes are two statements.
    let record = super::supervision::load(client).await?;
    let mut notes = Vec::new();
    let mut withheld_on_linked = 0;
    for r in client.query(NOTES_SQL, &[&members]).await? {
        // The chart is read before the row is parsed: a withheld note is only counted.
        if doubted && r.get::<_, String>(1).parse::<Uuid>()? != patient {
            withheld_on_linked += 1;
            continue;
        }
        notes.push(note_row(&r, &record)?);
    }
    // Ordered here rather than by SQL ORDER BY (ADR-0045); the id breaks a tie stably.
    notes.sort_by_key(|n| std::cmp::Reverse((n.t_effective, n.event_id)));
    let (discards, discards_withheld_on_linked) =
        discard_markers(client, patient, &members, doubted).await?;
    Ok(PatientNoteList {
        notes,
        withheld_on_linked,
        drafts: open_drafts(client, patient).await?,
        discards,
        discards_withheld_on_linked,
    })
}

#[cfg(test)]
mod tests {
    use super::{by_discarder, NOTES_BY_KEYS_SQL, NOTES_SQL};

    /// PR #984 review: the marker reads the writer word back, never defaults it.
    #[test]
    fn a_writer_word_this_build_does_not_know_is_an_error() {
        assert!(by_discarder(Some("discarder")).unwrap());
        assert!(!by_discarder(Some("not-known")).unwrap());
        for unknown in [None, Some(""), Some("colleague")] {
            assert!(by_discarder(unknown).is_err(), "{unknown:?}");
        }
    }

    /// #1021: the facts about a KEY (who holds it, their name, pinned role, confirmation, whether
    /// it is a human's) are looked up once per DISTINCT key, in the `facts` CTE, never once per
    /// row. Measured: at 100 000 notes over 40 keys the per-row calls were 88% of the menu's
    /// 10.4 s counts read. So outside the vouch column (whose calls run once per VOUCH, a few
    /// rows) each key-fact function is named exactly once, and the verified attester is computed
    /// once per row (it was twice: its own column and the lateral `k`).
    #[test]
    fn key_facts_are_looked_up_once_per_key_not_once_per_row() {
        const KEY_FACTS: [&str; 5] = [
            "cairn_note_signer_name(",
            "cairn_note_key_actor(",
            "cairn_note_key_role(",
            "cairn_key_confirmation(",
            "cairn_note_key_is_human(",
        ];
        for sql in [NOTES_SQL, NOTES_BY_KEYS_SQL] {
            let outside_vouches = sql.replace(crate::note::vouch_read::VOUCHES_SQL, "");
            assert!(sql.contains("facts AS MATERIALIZED"), "{sql}");
            for f in KEY_FACTS {
                assert_eq!(outside_vouches.matches(f).count(), 1, "{f} in {sql}");
            }
            assert_eq!(
                outside_vouches
                    .matches("cairn_note_vouched_attester(e.event_id)")
                    .count(),
                1,
                "{sql}"
            );
        }
    }

    /// #412 guard (PR #931 review): the note list reads the attester ONLY through the vouch
    /// door. Reading `event_log.attester_key` directly would mint a `VerifiedKid` from a peer's
    /// unverified token and grade a note "signed by" someone nobody vouched for. (A DB test
    /// cannot show this on the window's own notes: their human author is the signer, verified
    /// by the signature itself, so they carry no separate attester.)
    #[test]
    fn the_attester_is_read_only_through_the_vouch_door() {
        // Both expansions of `notes_sql!` (by chart, by keys) share the guard.
        for sql in [NOTES_SQL, NOTES_BY_KEYS_SQL] {
            assert!(sql.contains("cairn_note_vouched_attester(e.event_id)"));
            assert!(
                !sql.contains("attester_key"),
                "the raw attester column must never reach the classifier: {sql}"
            );
        }
    }
}
