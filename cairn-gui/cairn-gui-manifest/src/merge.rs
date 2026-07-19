//! The self-repairing merge: the user layer may only arrange/choose among what the
//! site layer offers; invalid or missing values fall back to site defaults. This
//! keeps soft policy within soft policy (the hard access gate is the DB floor).
use crate::model::{EffectiveManifest, SiteManifest, UserPrefs};
use cairn_gui_tab::TabId;

/// Clamp a divider ratio into a sane, both-panes-visible range; NaN → 0.5.
pub fn repair_ratio(r: f32) -> f32 {
    if r.is_nan() {
        0.5
    } else {
        r.clamp(0.1, 0.9)
    }
}

/// Keep only tabs the site offers, preserving the user's order; if the result is
/// empty, fall back to the site default — itself validated against `offered` (#213:
/// a site manifest whose default names an unoffered tab must not surface exactly
/// the tab this merge exists to filter; repair to the first offered tab instead).
/// A manifest offering NOTHING is broken beyond soft repair; the default is kept
/// then so the shell still renders something rather than panicking on empty panes.
fn filter_to_offered(user: &[TabId], offered: &[TabId], fallback: &TabId) -> Vec<TabId> {
    let filtered: Vec<TabId> = user
        .iter()
        .filter(|t| offered.contains(t))
        .cloned()
        .collect();
    if !filtered.is_empty() {
        return filtered;
    }
    if offered.contains(fallback) {
        vec![fallback.clone()]
    } else {
        vec![offered.first().unwrap_or(fallback).clone()]
    }
}

pub fn merge(site: &SiteManifest, user: &UserPrefs) -> EffectiveManifest {
    let left_tabs = filter_to_offered(&user.left_tabs, &site.offered, &site.default_left);
    let right_tabs = filter_to_offered(&user.right_tabs, &site.offered, &site.default_right);

    // Active tab: honour the user's choice only if it survived filtering, else the
    // first tab in that pane.
    let active_left = user
        .active_left
        .clone()
        .filter(|t| left_tabs.contains(t))
        .unwrap_or_else(|| left_tabs[0].clone());
    let active_right = user
        .active_right
        .clone()
        .filter(|t| right_tabs.contains(t))
        .unwrap_or_else(|| right_tabs[0].clone());

    // Rail is site-controlled but never shows an unoffered tab.
    let rail = site
        .rail
        .iter()
        .filter(|t| site.offered.contains(t))
        .cloned()
        .collect();

    EffectiveManifest {
        rail,
        divider_ratio: repair_ratio(user.divider_ratio),
        left_tabs,
        right_tabs,
        active_left,
        active_right,
    }
}

#[cfg(test)]
mod tests {
    use super::{merge, repair_ratio};
    use crate::model::{SiteManifest, UserPrefs};
    use cairn_gui_tab::TabId;

    fn site() -> SiteManifest {
        SiteManifest {
            offered: vec![TabId("note".into()), TabId("demographics".into())],
            rail: vec![TabId("note".into()), TabId("demographics".into())],
            default_left: TabId("note".into()),
            default_right: TabId("demographics".into()),
        }
    }

    #[test]
    fn user_cannot_surface_a_tab_the_site_does_not_offer() {
        let mut prefs = UserPrefs::default();
        prefs.left_tabs = vec![TabId("billing".into())]; // not offered
        let eff = merge(&site(), &prefs);
        assert!(
            !eff.left_tabs.contains(&TabId("billing".into())),
            "unoffered tab must be dropped (soft policy stays within soft policy)"
        );
    }

    #[test]
    fn empty_user_prefs_fall_back_to_site_defaults() {
        let eff = merge(&site(), &UserPrefs::default());
        assert_eq!(eff.active_left, TabId("note".into()));
        assert_eq!(eff.active_right, TabId("demographics".into()));
    }

    #[test]
    fn divider_ratio_is_clamped() {
        assert_eq!(repair_ratio(0.0), 0.1);
        assert_eq!(repair_ratio(5.0), 0.9);
        assert_eq!(repair_ratio(f32::NAN), 0.5);
        assert!((repair_ratio(0.4) - 0.4).abs() < f32::EPSILON);
    }

    #[test]
    fn an_unoffered_site_default_never_surfaces() {
        // #213 — the self-repairing merge existed to keep unoffered tabs out, but a
        // site manifest whose default_left names an UNOFFERED tab made the fallback
        // path surface exactly the tab it filters: empty user prefs → fall back to
        // default_left → the unoffered tab renders. The fallback must be validated
        // against `offered` like every user value is.
        let mut s = site();
        s.default_left = TabId("billing".into()); // site error: default not offered
        let eff = merge(&s, &UserPrefs::default());
        assert!(
            !eff.left_tabs.contains(&TabId("billing".into())),
            "an unoffered site default must not surface through the fallback"
        );
        assert_eq!(
            eff.left_tabs,
            vec![TabId("note".into())],
            "the repair falls back to the first offered tab"
        );
        assert_eq!(eff.active_left, TabId("note".into()));
    }

    #[test]
    fn user_may_reorder_offered_tabs() {
        let mut prefs = UserPrefs::default();
        prefs.left_tabs = vec![TabId("demographics".into()), TabId("note".into())];
        let eff = merge(&site(), &prefs);
        assert_eq!(
            eff.left_tabs,
            vec![TabId("demographics".into()), TabId("note".into())]
        );
    }
}
