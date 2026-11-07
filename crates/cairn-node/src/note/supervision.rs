//! The supervision record, read once with a chart's notes and used to annotate each part.
//!
//! Why: "does this part still wait for a co-sign" depends on the RECORD as it stood when the part
//! (or a vouch) entered the log, not as it stands now (sign-in slice 3c-3a). `list_patient_notes`
//! loads `cairn_supervision_record()` once and `annotate` stamps each part and vouch with plain
//! booleans; `cosign` then writes the rules over them. Every boolean defaults to today's
//! behaviour, so a part with no writer, or a record with no rows, changes nothing.
//!
//! "As of t" = the latest row of the op's family for that actor (and, for a link, that supervisor)
//! with `recorded_at <= t`. The door returns rows in `(recorded_at, seq)` order, so the latest
//! match is the LAST one in iteration order (a tie on `recorded_at` takes the later row).
use anyhow::{bail, Context};
use chrono::{DateTime, Utc};
use tokio_postgres::GenericClient;

use super::read::NoteRow;
use super::vouch::same_actor;

/// Which stream of rows an op belongs to; rows of one family supersede each other.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Family {
    Link,
    Cosign,
    Assigner,
}

/// The family of an op word. Exhaustive on purpose: a word this build does not know is an
/// error, never silently ignored (a new op must decide how annotation treats it).
fn family(op: &str) -> anyhow::Result<Family> {
    match op {
        "link-supervisor" | "unlink-supervisor" => Ok(Family::Link),
        "require-cosign" | "lift-cosign" => Ok(Family::Cosign),
        "grant-assigner" | "withdraw-assigner" => Ok(Family::Assigner),
        other => bail!("a supervision op is {other:?}, which this build does not know"),
    }
}

/// One row of the supervision record, as `cairn_supervision_record()` returns it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SupervisionRow {
    pub op: String,
    /// The subject (for a link: the SUPERVISEE), lower hex.
    pub actor: String,
    /// The supervisor, for link rows only.
    pub related: Option<String>,
    pub at: DateTime<Utc>,
}

/// The whole record, in the door's order.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct Supervision {
    rows: Vec<SupervisionRow>,
}

impl Supervision {
    /// Validate every op word up front, so a bad row fails the read rather than a later query.
    ///
    /// Each row's `at` is floored to the MILLISECOND. A part's or vouch's time is an HLC wall in
    /// milliseconds, but a registry row's `recorded_at` has microseconds: a requirement recorded at
    /// 10:00:00.000700 and a part signed in the same millisecond (wall .000) must count as
    /// "required", not "signed before the requirement". Comparing at the coarser clock's
    /// resolution makes a same-millisecond row count as in effect.
    pub fn from_rows(mut rows: Vec<SupervisionRow>) -> anyhow::Result<Self> {
        for r in &mut rows {
            family(&r.op)?;
            r.at = DateTime::from_timestamp_millis(r.at.timestamp_millis()).unwrap_or(r.at);
        }
        Ok(Self { rows })
    }

    /// The record said `actor` must have every note co-signed, as of `t`.
    pub fn needs_cosigning_at(&self, actor: &str, t: DateTime<Utc>) -> bool {
        self.rows
            .iter()
            .rfind(|r| {
                r.at <= t
                    && same_actor(&r.actor, actor)
                    && matches!(family(&r.op), Ok(Family::Cosign))
            })
            .is_some_and(|r| r.op == "require-cosign")
    }

    /// Supervisors whose link to `actor` stands after every row with `at <= t` (`None` = all rows).
    fn standing(&self, actor: &str, t: Option<DateTime<Utc>>) -> Vec<String> {
        let mut out: Vec<String> = Vec::new();
        for r in &self.rows {
            let related = r.related.as_deref().unwrap_or("");
            if t.is_some_and(|t| r.at > t)
                || !same_actor(&r.actor, actor)
                || !matches!(family(&r.op), Ok(Family::Link))
                || related.is_empty()
            {
                continue;
            }
            out.retain(|s| s != related);
            if r.op == "link-supervisor" {
                out.push(related.to_string());
            }
        }
        out
    }

    /// `supervisor` supervised `supervisee` as of `t`.
    pub fn supervises_at(&self, supervisor: &str, supervisee: &str, t: DateTime<Utc>) -> bool {
        self.standing(supervisee, Some(t))
            .iter()
            .any(|s| same_actor(s, supervisor))
    }

    /// `actor` had at least one supervisor as of `t`.
    pub fn has_supervisor_at(&self, actor: &str, t: DateTime<Utc>) -> bool {
        !self.standing(actor, Some(t)).is_empty()
    }

    /// `actor`'s supervisors now (after the last row).
    pub fn supervisors_now(&self, actor: &str) -> Vec<String> {
        self.standing(actor, None)
    }

    /// Every actor `supervisor` supervises now, sorted and without repeats (the menu's group 3
    /// reads their notes). The inverse of `supervisors_now`, by construction.
    pub fn supervisees_now(&self, supervisor: &str) -> Vec<String> {
        let mut actors: Vec<String> = self.rows.iter().map(|r| r.actor.clone()).collect();
        actors.sort();
        actors.dedup();
        actors.retain(|a| {
            self.supervisors_now(a)
                .iter()
                .any(|s| same_actor(s, supervisor))
        });
        actors
    }
}

/// Read the record through its door, on the caller's client (so the caller's snapshot).
pub async fn load(client: &(impl GenericClient + Sync)) -> anyhow::Result<Supervision> {
    let rows = client
        .query(
            "SELECT op, actor_id, related_actor, recorded_at FROM cairn_supervision_record()",
            &[],
        )
        .await
        .context("reading the supervision record")?;
    Supervision::from_rows(
        rows.iter()
            .map(|r| SupervisionRow {
                op: r.get(0),
                actor: r.get(1),
                related: r.get(2),
                at: r.get(3),
            })
            .collect(),
    )
}

/// Who wrote the part: its signer's actor, else (it went in unsigned) its drafted actor. `None`
/// for a signer this node cannot place (empty actor): who wrote it cannot be told, so no record is
/// judged, as `cosign::writer_signature` judges no co-sign rule for it.
pub fn writer_actor(p: &NoteRow) -> Option<&str> {
    match &p.signed_by {
        Some(s) => Some(s.actor.as_str()),
        None => p.drafted_actor.as_deref(),
    }
    .filter(|a| !a.is_empty())
}

/// Stamp the part and each of its vouches with the record's facts for the part's writer. The
/// requirement is judged at the PART's time, a vouch's supervisor at the VOUCH's own time, and
/// supervisors-now is just that. A part with no writer keeps every default.
pub fn annotate(p: &mut NoteRow, sup: &Supervision) {
    let Some(writer) = writer_actor(p).map(str::to_string) else {
        return;
    };
    p.needs_cosigning = sup.needs_cosigning_at(&writer, p.recorded_at);
    p.writer_supervisors = sup.supervisors_now(&writer);
    for v in &mut p.vouches {
        v.by_supervisor = sup.supervises_at(&v.by.actor, &writer, v.at);
        v.writer_supervised = sup.has_supervisor_at(&writer, v.at);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::note::vouch::fixture::{part, vouch, who};
    use cairn_event::note_attestation::{VouchAct, VouchRole};

    fn at(m: i64) -> DateTime<Utc> {
        DateTime::from_timestamp_millis(m * 60_000).unwrap()
    }
    fn row(op: &str, actor: &str, related: Option<&str>, m: i64) -> SupervisionRow {
        SupervisionRow {
            op: op.into(),
            actor: actor.into(),
            related: related.map(Into::into),
            at: at(m),
        }
    }
    fn sup(rows: Vec<SupervisionRow>) -> Supervision {
        Supervision::from_rows(rows).unwrap()
    }

    #[test]
    fn supervisees_now_lists_each_standing_supervisee_once() {
        let s = sup(vec![
            row("link-supervisor", "b", Some("a"), 1),
            row("link-supervisor", "c", Some("a"), 2),
            row("unlink-supervisor", "c", Some("a"), 3),
            row("link-supervisor", "c", Some("a"), 4),
            row("link-supervisor", "d", Some("e"), 5),
        ]);
        assert_eq!(s.supervisees_now("a"), ["b", "c"]);
        assert_eq!(s.supervisees_now("e"), ["d"]);
        assert!(s.supervisees_now("zz").is_empty());
    }

    #[test]
    fn the_requirement_is_read_as_it_stood_at_t() {
        let s = sup(vec![
            row("require-cosign", "aa", None, 10),
            row("lift-cosign", "aa", None, 20),
        ]);
        assert!(s.needs_cosigning_at("aa", at(15)));
        // `<=`: a row recorded exactly at t is in effect at t.
        assert!(sup(vec![row("require-cosign", "aa", None, 10)]).needs_cosigning_at("aa", at(10)));
        assert!(!s.needs_cosigning_at("aa", at(25)));
        assert!(!s.needs_cosigning_at("aa", at(5)));
        assert!(!s.needs_cosigning_at("bb", at(15)));
    }

    #[test]
    fn a_link_is_read_as_it_stood_at_t() {
        let s = sup(vec![
            row("link-supervisor", "aa", Some("sv"), 10),
            row("unlink-supervisor", "aa", Some("sv"), 20),
            row("link-supervisor", "aa", Some("s2"), 30),
        ]);
        assert!(!s.supervises_at("sv", "aa", at(5)));
        assert!(s.supervises_at("sv", "aa", at(15)));
        assert!(
            s.supervises_at("sv", "aa", at(10)),
            "`<=`: a link at t is in effect at t"
        );
        assert!(!s.supervises_at("sv", "aa", at(25)));
        assert!(!s.has_supervisor_at("aa", at(25)));
        assert!(s.has_supervisor_at("aa", at(35)));
        assert_eq!(s.supervisors_now("aa"), vec!["s2".to_string()]);
        // The link is on the supervisee, not the supervisor.
        assert!(!s.supervises_at("aa", "sv", at(15)));
    }

    /// A registry row has microseconds, a part's wall only milliseconds: same-millisecond counts.
    #[test]
    fn a_row_in_the_same_millisecond_as_the_part_is_in_effect() {
        let mut r = row("require-cosign", "aa", None, 0);
        r.at = DateTime::from_timestamp_micros(1_000_000_700).unwrap(); // .000700
        let s = sup(vec![r]);
        assert!(s.needs_cosigning_at("aa", DateTime::from_timestamp_millis(1_000_000).unwrap()));
    }

    #[test]
    fn a_tie_on_recorded_at_takes_the_later_row() {
        let s = sup(vec![
            row("require-cosign", "aa", None, 10),
            row("lift-cosign", "aa", None, 10),
        ]);
        assert!(!s.needs_cosigning_at("aa", at(10)));
    }

    #[test]
    fn an_unknown_op_word_is_an_error() {
        assert!(Supervision::from_rows(vec![row("promote", "aa", None, 1)]).is_err());
        // The assigner family is irrelevant to annotation but must parse.
        assert!(Supervision::from_rows(vec![row("grant-assigner", "aa", None, 1)]).is_ok());
    }

    #[test]
    fn an_empty_actor_is_nobodys_supervisor() {
        let s = sup(vec![
            row("link-supervisor", "aa", Some("sv"), 1),
            row("link-supervisor", "", Some("sv"), 1),
            row("require-cosign", "", None, 1),
        ]);
        assert!(!s.supervises_at("", "aa", at(5)));
        assert!(!s.has_supervisor_at("", at(5)));
        assert!(!s.needs_cosigning_at("", at(5)));
        assert!(s.supervisors_now("").is_empty());
    }

    /// A part judged at ITS time: required then, lifted since, still waits (Review Focus 1).
    #[test]
    fn annotate_judges_the_requirement_at_the_parts_time() {
        let s = sup(vec![
            row("require-cosign", "aa", None, 0),
            row("lift-cosign", "aa", None, 60 * 24 * 365 * 100),
        ]);
        let mut p = part(1, Some(who("aa", "Dr A")));
        annotate(&mut p, &s);
        assert!(p.needs_cosigning);
        let mut early = part(2, Some(who("aa", "Dr A")));
        early.recorded_at = at(-5);
        annotate(&mut early, &s);
        assert!(!early.needs_cosigning);
    }

    /// Review Focus 2: linked after the vouch is false; unlinked after it is still true.
    #[test]
    fn annotate_judges_each_vouchs_supervisor_at_its_own_time() {
        let base = part(1, Some(who("aa", "Dr A"))).recorded_at;
        let ms = |m: i64| base + chrono::Duration::minutes(m);
        let s = Supervision::from_rows(vec![
            SupervisionRow {
                op: "link-supervisor".into(),
                actor: "aa".into(),
                related: Some("s1".into()),
                at: ms(-10),
            },
            SupervisionRow {
                op: "unlink-supervisor".into(),
                actor: "aa".into(),
                related: Some("s1".into()),
                at: ms(10),
            },
            SupervisionRow {
                op: "link-supervisor".into(),
                actor: "aa".into(),
                related: Some("s2".into()),
                at: ms(10),
            },
        ])
        .unwrap();
        let mut p = part(1, Some(who("aa", "Dr A")));
        for (n, by) in [(10, "s1"), (11, "s2")] {
            let mut v = vouch(n, VouchAct::SignOff, VouchRole::CoSigned, who(by, "S"));
            v.at = base;
            p.vouches.push(v);
        }
        annotate(&mut p, &s);
        assert!(
            p.vouches[0].by_supervisor,
            "s1 stepped away later, still counts"
        );
        assert!(
            !p.vouches[1].by_supervisor,
            "s2 was linked only after the vouch"
        );
        assert!(p.vouches.iter().all(|v| v.writer_supervised));
        assert_eq!(p.writer_supervisors, vec!["s2".to_string()]);

        // A vouch dated before EVERY link: the writer had no supervisor then (the fallback), even
        // though one is linked now. "At the vouch's time" is not "now".
        let mut early = part(2, Some(who("aa", "Dr A")));
        let mut v = vouch(20, VouchAct::SignOff, VouchRole::CoSigned, who("s2", "S"));
        v.at = ms(-60);
        early.vouches.push(v);
        annotate(&mut early, &s);
        assert!(!early.writer_supervisors.is_empty());
        assert!(!early.vouches[0].writer_supervised);
        assert!(!early.vouches[0].by_supervisor);
    }

    /// A signer this node cannot place (empty actor) leaves the writer unknown: the drafted actor's
    /// record is NOT judged, as `cosign::writer_signature` judges no co-sign rule for it.
    #[test]
    fn an_unplaceable_signer_is_no_writer() {
        let mut p = part(1, Some(who("", "a key this node does not know")));
        p.drafted_actor = Some("aa".into());
        assert_eq!(writer_actor(&p), None);
        let s = sup(vec![row("require-cosign", "aa", None, -1)]);
        let before = p.clone();
        annotate(&mut p, &s);
        assert_eq!(p, before);
    }

    #[test]
    fn annotate_leaves_a_part_with_no_writer_alone() {
        let s = sup(vec![row("require-cosign", "aa", None, -1)]);
        let mut p = part(1, None);
        let before = p.clone();
        annotate(&mut p, &s);
        assert_eq!(p, before);
    }
}
