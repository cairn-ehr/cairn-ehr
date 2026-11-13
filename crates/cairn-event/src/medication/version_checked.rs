//! `clinical.medication-set-aside-checked.asserted` — "checked: this set-aside version is NOT current".
//!
//! Pure: no clock, no I/O. A clinician who looks again at a set-aside version (the packet, the
//! pharmacy, the daughter) and finds it still not current records that here, so the next
//! reader sees the check was done rather than wondering.
//!
//! Why this is its own type with NO verdict field: the type IS the verdict. "Checked: not
//! current" is the only thing it can say. The opposite outcome ("it was current after all")
//! is not a flag here but a restore: a new charted line carrying `restores` (spec §3.4), so
//! there is one wire spelling for each outcome and no `verdict: bool` to get backwards.
use serde_json::{json, Value};

use super::source_note::{insert_source_note, SourceNote};

/// The event type of a "checked: not current" assertion about one set-aside version.
pub const VERSION_CHECKED_EVENT_TYPE: &str = "clinical.medication-set-aside-checked.asserted";
/// The payload's schema tag, class `additive`.
pub const VERSION_CHECKED_SCHEMA: &str = "clinical.medication-set-aside-checked/1";

/// One check of the set-aside version `version_id`.
#[derive(Debug)]
pub struct VersionChecked<'a> {
    pub version_id: &'a str,
    /// The source category of the check (same open word list as a charted line's).
    pub info_source: &'a str,
    /// Optional finer provenance (`source_detail`) and rationale (`why`).
    pub source_note: Option<SourceNote<'a>>,
}

/// The payload: `{version_id, info_source}` plus the source-note keys when present.
pub fn version_checked_body(c: &VersionChecked) -> Value {
    let mut v = json!({ "version_id": c.version_id, "info_source": c.info_source });
    if let Some(n) = &c.source_note {
        insert_source_note(v.as_object_mut().expect("json! built an object"), n);
    }
    v
}

/// `Set-aside version checked: not current — from {source}{ — {why}}`, where `{source}` is the
/// `source_detail` if present, else the `info_source` word. Word order is frozen (twin = wire).
pub fn render_version_checked_twin(c: &VersionChecked) -> String {
    let note = c.source_note.unwrap_or_default();
    let from = note.source_detail.unwrap_or(c.info_source);
    let mut t = format!("Set-aside version checked: not current — from {from}");
    if let Some(w) = note.why {
        t.push_str(&format!(" — {w}"));
    }
    t
}

#[cfg(test)]
mod tests {
    use super::*;

    const V: &str = "22222222-2222-7222-8222-222222222222";

    fn checked(note: Option<SourceNote<'static>>) -> VersionChecked<'static> {
        VersionChecked {
            version_id: V,
            info_source: "external-record",
            source_note: note,
        }
    }

    #[test]
    fn the_checked_body_is_version_id_info_source_and_the_note() {
        let c = checked(Some(SourceNote {
            source_detail: Some("pharmacy"),
            why: Some("dispensed 25 mg"),
        }));
        assert_eq!(
            version_checked_body(&c),
            json!({"version_id": V, "info_source": "external-record",
                   "source_detail": "pharmacy", "why": "dispensed 25 mg"})
        );
    }

    #[test]
    fn a_none_note_key_is_absent() {
        let c = checked(Some(SourceNote {
            source_detail: Some("pharmacy"),
            why: None,
        }));
        let v = version_checked_body(&c);
        assert!(!v.as_object().unwrap().contains_key("why"));
        let bare = version_checked_body(&checked(None));
        assert_eq!(
            bare,
            json!({"version_id": V, "info_source": "external-record"})
        );
    }

    #[test]
    fn the_checked_twin_names_the_source_and_the_why() {
        let c = checked(Some(SourceNote {
            source_detail: Some("pharmacy"),
            why: Some("dispensed 25 mg"),
        }));
        assert_eq!(
            render_version_checked_twin(&c),
            "Set-aside version checked: not current — from pharmacy — dispensed 25 mg"
        );
    }

    #[test]
    fn falls_back_to_the_info_source_word() {
        assert_eq!(
            render_version_checked_twin(&checked(None)),
            "Set-aside version checked: not current — from external-record"
        );
    }
}
