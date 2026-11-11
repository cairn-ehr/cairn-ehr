//! The one read behind the person menu (sign-in slice 3c-3c): the bar asks for ONE person's
//! menu, the picker for everyone's counts. Each is its own [`MenuRead`] (its own snapshot), and
//! both go through the same [`classify`], so within one read a count and its list can never
//! disagree; the bar's read and the picker's are two reads (ADR-0095 decision 6).
//!
//! The reads run in this order, all on the CALLER's client: people, then the supervision record,
//! then the drafts, then the medication lines, then the parts. The caller supplies a REPEATABLE READ snapshot (as `note_list`
//! does); this module opens no transaction of its own.
use anyhow::Context;
use chrono::{DateTime, Utc};
use tokio_postgres::GenericClient;
use uuid::Uuid;

use super::menu::{
    add_medication_lines, classify, review_window_start, MedLines, MenuDraft, Person, PersonMenu,
};
use super::read::{parts_by_keys, NoteRow};
use super::supervision::{self, Supervision};
use super::vouch::same_actor;

/// Everything read for the menu, plus what is needed to answer questions about it.
pub struct MenuRead {
    pub people: Vec<Person>,
    pub drafts: Vec<MenuDraft>,
    /// Unsigned medication lines by writer and chart (`medication::draft::writer_drafts`).
    pub med_drafts: Vec<MedLines>,
    pub parts: Vec<NoteRow>,
    /// The instant the parts were read for. [`MenuRead::menu_for`] and [`MenuRead::counts`] judge
    /// at this instant and no other: the read skipped every part no menu could list at it (#1021),
    /// so judging at an earlier instant could silently miss a part of the review group.
    pub read_at: DateTime<Utc>,
}

/// `Some(staff)`: that person's keys and their current supervisees' keys only (the bar).
/// `None`: every person's keys (the picker's counts).
///
/// `now` is the instant the menus are judged at, kept as [`MenuRead::read_at`]: parts that no menu
/// could list at `now` are not returned (they are still scanned, `read::NOTES_BY_KEYS_SQL`, #1021).
///
/// `staff` is normalised by the database (`cairn_vault_staff_id`), never by a copy of its rule
/// here. A staff id that names no person yields a `MenuRead` without that person and with no
/// parts read, so [`MenuRead::menu_for`] answers `None`.
pub async fn read_menus(
    client: &(impl GenericClient + Sync),
    scope: Option<&str>,
    now: DateTime<Utc>,
) -> anyhow::Result<MenuRead> {
    let people = read_people(client).await?;
    let sup = supervision::load(client).await?;
    let mut drafts = read_drafts(client).await?;
    let mut med_drafts = crate::medication::draft::writer_drafts(client).await?;

    let keys = match scope {
        None => people.iter().flat_map(|p| p.keys.clone()).collect(),
        Some(raw) => {
            // The database owns the normalisation rule; Rust only compares its answer.
            let staff: String = client
                .query_one("SELECT cairn_vault_staff_id($1)", &[&raw])
                .await
                .context("normalising the staff id")?
                .get(0);
            drafts.retain(|d| d.writer == staff);
            med_drafts.retain(|d| d.0 == staff);
            keys_for_bar(&people, &staff, &sup)
        }
    };
    // No person matched: no keys, so nothing to read.
    let parts = if keys.is_empty() {
        Vec::new()
    } else {
        parts_by_keys(client, &keys, &sup, review_window_start(now)).await?
    };
    Ok(MenuRead {
        people,
        drafts,
        med_drafts,
        parts,
        read_at: now,
    })
}

/// The keys the bar needs: the named person's own and those of everyone they supervise now
/// (group 3 reads their notes). Empty when `staff` names no one. Names are NOT narrowed:
/// `people` stays whole so a row can name its writer and the other supervisors.
fn keys_for_bar(people: &[Person], staff: &str, sup: &Supervision) -> Vec<String> {
    let Some(me) = find_person(people, staff) else {
        return Vec::new();
    };
    let supervisees = sup.supervisees_now(&me.actor);
    people
        .iter()
        .filter(|p| p.actor == me.actor || supervisees.iter().any(|s| same_actor(s, &p.actor)))
        .flat_map(|p| p.keys.clone())
        .collect()
}

/// The person with this staff id; `""` names nobody (a person read without a staff id has it).
/// The bar's keys and [`MenuRead::person`] both ask here, so the two cannot pick differently.
fn find_person<'a>(people: &'a [Person], staff: &str) -> Option<&'a Person> {
    if staff.is_empty() {
        return None;
    }
    people.iter().find(|p| p.staff_id == staff)
}

async fn read_people(client: &(impl GenericClient + Sync)) -> anyhow::Result<Vec<Person>> {
    let rows = client
        .query(
            "SELECT actor_id, staff_id, display_name, keys FROM cairn_note_people()",
            &[],
        )
        .await
        .context("reading the people the menu may name")?;
    Ok(rows
        .iter()
        .map(|r| Person {
            actor: r.get(0),
            // NULL when an actor the supervision record names has no handle to answer with;
            // "" is nobody's staff id (`MenuRead::person`, `counts`).
            staff_id: r.get::<_, Option<String>>(1).unwrap_or_default(),
            display_name: r.get(2),
            keys: r.get(3),
        })
        .collect())
}

async fn read_drafts(client: &(impl GenericClient + Sync)) -> anyhow::Result<Vec<MenuDraft>> {
    let rows = client
        .query(
            "SELECT writer, draft_id::text, patient_id::text, due, stuck \
               FROM cairn_note_drafts_by_writer()",
            &[],
        )
        .await
        .context("reading the open drafts by writer")?;
    rows.iter()
        .map(|r| {
            Ok(MenuDraft {
                writer: r.get(0),
                draft_id: r.get::<_, String>(1).parse::<Uuid>()?,
                chart: r.get::<_, String>(2).parse::<Uuid>()?,
                due: r.get(3),
                stuck: r.get(4),
            })
        })
        .collect()
}

impl MenuRead {
    /// The person with this (already normalised) staff id, or `None` for `""`.
    pub fn person(&self, staff_id: &str) -> Option<&Person> {
        find_person(&self.people, staff_id)
    }

    /// One person's groups: the note groups, then their unsigned medication lists.
    fn menu_of(&self, me: &Person) -> PersonMenu {
        let mut menu = classify(me, &self.parts, &self.drafts, self.read_at);
        add_medication_lines(&mut menu, me, &self.med_drafts);
        menu
    }

    /// That person's three groups at [`MenuRead::read_at`], or `None` when no such person was read.
    pub fn menu_for(&self, staff_id: &str) -> Option<PersonMenu> {
        let me = self.person(staff_id)?;
        Some(self.menu_of(me))
    }

    /// One `(staff id, count)` per person with a staff id, in people order, at [`MenuRead::read_at`].
    pub fn counts(&self) -> Vec<(String, usize)> {
        self.people
            .iter()
            .filter(|p| !p.staff_id.is_empty())
            .map(|p| (p.staff_id.clone(), self.menu_of(p).count()))
            .collect()
    }

    /// The display name of the person with this actor, if the node can name them.
    pub fn name_of(&self, actor: &str) -> Option<&str> {
        self.people
            .iter()
            .find(|p| p.actor == actor)
            .and_then(|p| p.display_name.as_deref())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// F5 (final review): an actor the supervision record names but no staff id can be found for
    /// reads as staff id "" (`cairn_note_people()` answers NULL). "" is nobody's name, so it has
    /// no menu, just as `counts()` gives it no count.
    #[test]
    fn a_person_without_a_staff_id_has_no_menu() {
        let read = MenuRead {
            people: vec![Person {
                actor: "aa".into(),
                staff_id: String::new(),
                display_name: None,
                keys: vec!["k".into()],
            }],
            drafts: Vec::new(),
            med_drafts: Vec::new(),
            parts: Vec::new(),
            read_at: Utc::now(),
        };
        assert!(read.person("").is_none());
        assert!(read.menu_for("").is_none());
        assert!(read.counts().is_empty());
    }

    /// The picker's count includes unsigned medication lines (batch charting): it comes from the
    /// same builder as the bar's, so the two cannot disagree.
    #[test]
    fn the_picker_counts_include_medication_lines() {
        let person = |a: &str| Person {
            actor: a.into(),
            staff_id: format!("staff-{a}"),
            display_name: None,
            keys: vec![],
        };
        let read = MenuRead {
            people: vec![person("me"), person("you")],
            drafts: Vec::new(),
            med_drafts: vec![("staff-me".into(), Uuid::from_u128(1), 2, Utc::now())],
            parts: Vec::new(),
            read_at: Utc::now(),
        };
        assert_eq!(
            read.counts(),
            vec![("staff-me".to_string(), 1), ("staff-you".to_string(), 0)]
        );
        assert_eq!(read.menu_for("staff-me").unwrap().count(), 1);
    }
}
