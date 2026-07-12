//! The read-only context the shell hands to every tab. `capabilities` is the
//! ENTIRE seam to the future role/policy subsystem (spec §6): today a stub grants
//! a clinician everything; later the resolver is replaced and nothing else changes.
use std::collections::HashSet;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Capability {
    ViewDemographics,
    ViewNote,
    ViewMeds,
    ViewResults,
}

#[derive(Debug, Clone, Default)]
pub struct Capabilities {
    granted: HashSet<Capability>,
}

impl Capabilities {
    /// The slice-1 stub resolver: a clinician sees everything. The real resolver
    /// (subsystem B) replaces only this function.
    pub fn clinician_all() -> Self {
        Self {
            granted: [
                Capability::ViewDemographics,
                Capability::ViewNote,
                Capability::ViewMeds,
                Capability::ViewResults,
            ]
            .into_iter()
            .collect(),
        }
    }

    pub fn allows(&self, cap: Capability) -> bool {
        self.granted.contains(&cap)
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PatientRef {
    pub uuid: String,
    pub display_name: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct UserRef {
    pub actor_id: String,
    pub display_name: String,
}

#[derive(Debug, Clone)]
pub struct Context {
    pub patient: Option<PatientRef>,
    pub user: UserRef,
    pub capabilities: Capabilities,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn clinician_stub_grants_all_known_capabilities() {
        let caps = Capabilities::clinician_all();
        for cap in [
            Capability::ViewDemographics,
            Capability::ViewNote,
            Capability::ViewMeds,
            Capability::ViewResults,
        ] {
            assert!(caps.allows(cap), "clinician stub should grant {cap:?}");
        }
    }

    #[test]
    fn empty_capabilities_deny() {
        let caps = Capabilities::default();
        assert!(!caps.allows(Capability::ViewMeds));
    }
}
