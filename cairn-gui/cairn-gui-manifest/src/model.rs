//! Manifest data model. Two layers (spec §7): the site/role layer (what tabs are
//! OFFERED — the seam to subsystem B) and the per-user layer (personal layout,
//! remember-last-state). Storage is deferred to a later slice (Postgres); this
//! slice merges/repairs in memory.
use cairn_gui_tab::TabId;

#[derive(Debug, Clone)]
pub struct SiteManifest {
    pub offered: Vec<TabId>,
    pub rail: Vec<TabId>,
    pub default_left: TabId,
    pub default_right: TabId,
}

#[derive(Debug, Clone)]
pub struct UserPrefs {
    pub divider_ratio: f32,
    pub left_tabs: Vec<TabId>,
    pub right_tabs: Vec<TabId>,
    pub active_left: Option<TabId>,
    pub active_right: Option<TabId>,
}

impl Default for UserPrefs {
    fn default() -> Self {
        Self {
            divider_ratio: 0.5,
            left_tabs: Vec::new(),
            right_tabs: Vec::new(),
            active_left: None,
            active_right: None,
        }
    }
}

#[derive(Debug, Clone)]
pub struct EffectiveManifest {
    pub rail: Vec<TabId>,
    pub divider_ratio: f32,
    pub left_tabs: Vec<TabId>,
    pub right_tabs: Vec<TabId>,
    pub active_left: TabId,
    pub active_right: TabId,
}
