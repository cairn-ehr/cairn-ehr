//! Pure pane/tab model + the cross-pane routing rule. No iced. This is where the
//! "open the reference in the OTHER pane, leaving the note in place" behaviour
//! (spec §5) lives, so it is unit-tested in isolation.
use cairn_gui_manifest::EffectiveManifest;
use cairn_gui_tab::TabId;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Side {
    Left,
    Right,
}

impl Side {
    pub fn opposite(self) -> Side {
        match self {
            Side::Left => Side::Right,
            Side::Right => Side::Left,
        }
    }
}

#[derive(Debug, Clone)]
struct Pane {
    tabs: Vec<TabId>,
    active: TabId,
}

#[derive(Debug, Clone)]
pub struct Workspace {
    left: Pane,
    right: Pane,
    pub divider_ratio: f32,
}

impl Workspace {
    pub fn from_manifest(m: &EffectiveManifest) -> Self {
        Self {
            left: Pane { tabs: m.left_tabs.clone(), active: m.active_left.clone() },
            right: Pane { tabs: m.right_tabs.clone(), active: m.active_right.clone() },
            divider_ratio: m.divider_ratio,
        }
    }

    fn pane(&self, side: Side) -> &Pane {
        match side {
            Side::Left => &self.left,
            Side::Right => &self.right,
        }
    }

    fn pane_mut(&mut self, side: Side) -> &mut Pane {
        match side {
            Side::Left => &mut self.left,
            Side::Right => &mut self.right,
        }
    }

    pub fn tabs(&self, side: Side) -> &[TabId] {
        &self.pane(side).tabs
    }

    pub fn active(&self, side: Side) -> &TabId {
        &self.pane(side).active
    }

    pub fn activate(&mut self, side: Side, tab: &TabId) {
        let p = self.pane_mut(side);
        if p.tabs.contains(tab) {
            p.active = tab.clone();
        }
    }

    /// Cross-pane routing (spec §5): open `tab` in the pane OPPOSITE `from`, adding
    /// it if absent, and make it active. Returns the pane it landed in. Leaving the
    /// originating pane untouched is the whole point (the note stays put).
    pub fn open_in_opposite(&mut self, from: Side, tab: TabId) -> Side {
        let target = from.opposite();
        let p = self.pane_mut(target);
        if !p.tabs.contains(&tab) {
            p.tabs.push(tab.clone());
        }
        p.active = tab;
        target
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use cairn_gui_manifest::EffectiveManifest;
    use cairn_gui_tab::TabId;

    fn eff() -> EffectiveManifest {
        EffectiveManifest {
            rail: vec![TabId("note".into()), TabId("demographics".into())],
            divider_ratio: 0.5,
            left_tabs: vec![TabId("note".into())],
            right_tabs: vec![TabId("demographics".into())],
            active_left: TabId("note".into()),
            active_right: TabId("demographics".into()),
        }
    }

    #[test]
    fn open_in_opposite_from_left_lands_on_right_and_activates() {
        let mut ws = Workspace::from_manifest(&eff());
        let landed = ws.open_in_opposite(Side::Left, TabId("xray".into()));
        assert_eq!(landed, Side::Right);
        assert_eq!(ws.active(Side::Right), &TabId("xray".into()));
        assert!(ws.tabs(Side::Right).contains(&TabId("xray".into())));
    }

    #[test]
    fn open_in_opposite_does_not_duplicate_an_existing_tab() {
        let mut ws = Workspace::from_manifest(&eff());
        ws.open_in_opposite(Side::Left, TabId("demographics".into())); // already on right
        let count = ws.tabs(Side::Right).iter().filter(|t| **t == TabId("demographics".into())).count();
        assert_eq!(count, 1, "no duplicate tab");
    }

    #[test]
    fn side_opposite() {
        assert_eq!(Side::Left.opposite(), Side::Right);
        assert_eq!(Side::Right.opposite(), Side::Left);
    }
}
