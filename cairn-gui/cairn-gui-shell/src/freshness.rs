//! Pure age rule for the stale FLAG. Deliberately minimal: the shell NEVER
//! silently swaps on-screen data (spec §6) — it uses this only to decide whether
//! to show the "stale — Refresh?" affordance on already-visible content.

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Freshness {
    Fresh,
    Stale,
}

#[derive(Debug, Clone, Copy)]
pub struct Loaded {
    pub at_tick: u64,
}

pub fn freshness(loaded: &Loaded, now_tick: u64, ttl: u64) -> Freshness {
    if now_tick.saturating_sub(loaded.at_tick) > ttl {
        Freshness::Stale
    } else {
        Freshness::Fresh
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn within_ttl_is_fresh() {
        assert_eq!(
            freshness(&Loaded { at_tick: 100 }, 150, 60),
            Freshness::Fresh
        );
    }

    #[test]
    fn beyond_ttl_is_stale() {
        assert_eq!(
            freshness(&Loaded { at_tick: 100 }, 200, 60),
            Freshness::Stale
        );
    }
}
