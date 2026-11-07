//! The one read behind the person menu (sign-in slice 3c-3c): the bar asks for ONE person's
//! menu, the picker for everyone's counts, and both come from the same [`MenuRead`] through
//! [`classify`], so a count and a list can never disagree.
//!
//! The reads run in this order, all on the CALLER's client: people, then the supervision record,
//! then the drafts, then the parts. The caller supplies a REPEATABLE READ snapshot (as `note_list`
//! does); this module opens no transaction of its own.
use anyhow::Context;
use chrono::{DateTime, Utc};
use tokio_postgres::GenericClient;
use uuid::Uuid;

use super::menu::{classify, MenuDraft, Person, PersonMenu};
use super::read::{parts_by_keys, NoteRow};
use super::supervision::{self, Supervision};
use super::vouch::same_actor;

/// Everything read for the menu, plus what is needed to answer questions about it.
pub struct MenuRead {
    pub people: Vec<Person>,
    pub drafts: Vec<MenuDraft>,
    pub parts: Vec<NoteRow>,
}

/// `Some(staff)`: that person's keys and their current supervisees' keys only (the bar).
/// `None`: every person's keys (the picker's counts).
///
/// `staff` is normalised by the database (`cairn_vault_staff_id`), never by a copy of its rule
/// here. A staff id that names no person yields a `MenuRead` without that person and with no
/// parts read, so [`MenuRead::menu_for`] answers `None`.
pub async fn read_menus(
    client: &(impl GenericClient + Sync),
    scope: Option<&str>,
) -> anyhow::Result<MenuRead> {
    let people = read_people(client).await?;
    let sup = supervision::load(client).await?;
    let mut drafts = read_drafts(client).await?;

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
            keys_for_bar(&people, &staff, &sup)
        }
    };
    // No person matched: no keys, so nothing to read.
    let parts = if keys.is_empty() {
        Vec::new()
    } else {
        parts_by_keys(client, &keys, &sup).await?
    };
    Ok(MenuRead {
        people,
        drafts,
        parts,
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

    /// That person's three groups, or `None` when no such person was read.
    pub fn menu_for(&self, staff_id: &str, now: DateTime<Utc>) -> Option<PersonMenu> {
        let me = self.person(staff_id)?;
        Some(classify(me, &self.parts, &self.drafts, now))
    }

    /// One `(staff id, count)` per person with a staff id, in people order.
    pub fn counts(&self, now: DateTime<Utc>) -> Vec<(String, usize)> {
        self.people
            .iter()
            .filter(|p| !p.staff_id.is_empty())
            .map(|p| {
                (
                    p.staff_id.clone(),
                    classify(p, &self.parts, &self.drafts, now).count(),
                )
            })
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
            parts: Vec::new(),
        };
        assert!(read.person("").is_none());
        assert!(read.menu_for("", Utc::now()).is_none());
        assert!(read.counts(Utc::now()).is_empty());
    }
}
