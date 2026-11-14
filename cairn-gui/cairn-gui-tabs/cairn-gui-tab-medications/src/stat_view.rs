//! The stat line under a drug line (charting slice 2, design §3.6): what a stat order says about
//! itself — when it was ordered and by whom, and whether, when and by whom it was given.
//!
//! On paper a stat order sits in the chart's "stat" box; the nurse who gives it initials it with
//! the time, and a dose initialled in error is struck through with the striker's initials, never
//! erased. Here the line says the same in words (Global Constraints' copy), and a struck give
//! keeps its own line under it.
//!
//! Kept apart from `row_view.rs` (which would pass 500 lines), and fed a plain owned input
//! ([`StatWordsIn`], ruling R3): this crate does not depend on `cairn-node`, so the window maps
//! each read `StatOrder` into it (`chart_stat::attach_stat`). Every function here is pure.
use crate::hold_view::local_instant;
use crate::row_view::SIGNER_NOT_KNOWN;
use chrono::{DateTime, TimeZone, Utc};
use serde::Serialize;
use std::fmt::Display;

/// The stat line as the window draws it under its drug line.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct StatView {
    /// The stat order's thread: what "Given" sends.
    pub medication_id: String,
    /// [`stat_words`]: the order and its standing gives.
    pub line: String,
    /// [`struck_words`]: one line per give entered in error, oldest first.
    pub struck: Vec<String>,
    /// "Given" is offered: the order is on the OPENED chart, its line is current, and it is
    /// waiting to be given. Rust re-derives all three on a fresh read before the key.
    pub can_give: bool,
    /// The standing (unstruck) gives, oldest first, each with its "Entered in error".
    pub gives: Vec<GiveView>,
}

/// One standing give of a stat order.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct GiveView {
    /// The given event's id: what "Entered in error" sends.
    pub event_id: String,
    /// "Entered in error" is offered: the give is on the OPENED chart (never a linked chart's).
    pub can_strike: bool,
}

/// A stat order, as its words need it (ruling R3).
#[derive(Debug, Clone)]
pub struct StatWordsIn {
    /// The order event's `t_recorded`.
    pub ordered_at: DateTime<Utc>,
    /// The orderer's name; `None` is "signer not known".
    pub orderer: Option<String>,
    pub gives: Vec<GiveIn>,
}

/// One give of a stat order.
#[derive(Debug, Clone)]
pub struct GiveIn {
    /// The time given (the given event's `t_effective`).
    pub given_at: DateTime<Utc>,
    pub giver: Option<String>,
    /// Entered in error: `(who, at, why)` — the striker's name, the strike's `t_recorded`, and
    /// the why when one was typed.
    pub struck: Option<(Option<String>, DateTime<Utc>, Option<String>)>,
}

/// The stat line's words from the order and its STANDING gives, in time order (Global
/// Constraints): `"stat · ordered {o} {orderer} · not yet given"`, or `"… · given {g} {giver}"`
/// per standing give. A give EARLIER than the order was charted (a verbal order written up after
/// the dose) reads `"given {g} {giver}, before it was charted {o} {orderer}"` in the order's place
/// (§10 Q3: shown, never refused, never reordered). Struck gives are left to [`struck_words`]:
/// the line says what stands. Pure.
pub fn stat_words<Tz: TimeZone>(o: &StatWordsIn, now: &DateTime<Tz>) -> String
where
    Tz::Offset: Display,
{
    let ordered = format!("{} {}", when(&o.ordered_at, now), name(&o.orderer));
    let mut standing: Vec<&GiveIn> = o.gives.iter().filter(|g| g.struck.is_none()).collect();
    standing.sort_by_key(|g| g.given_at);
    let mut parts = vec!["stat".to_string()];
    // "ordered …" stands on its own only when no give came before it; otherwise each give
    // before it carries it, as "before it was charted …".
    if standing.first().is_none_or(|g| g.given_at >= o.ordered_at) {
        parts.push(format!("ordered {ordered}"));
    }
    for g in &standing {
        let given = format!("given {} {}", when(&g.given_at, now), name(&g.giver));
        parts.push(if g.given_at < o.ordered_at {
            format!("{given}, before it was charted {ordered}")
        } else {
            given
        });
    }
    if standing.is_empty() {
        parts.push("not yet given".into());
    }
    parts.join(" · ")
}

/// One line per STRUCK give, oldest first (Global Constraints): `"given {g} {giver} — entered in
/// error by {who}, {when}"`, then `" — {why}"` when one was typed. Pure.
pub fn struck_words<Tz: TimeZone>(o: &StatWordsIn, now: &DateTime<Tz>) -> Vec<String>
where
    Tz::Offset: Display,
{
    let mut struck: Vec<_> = o
        .gives
        .iter()
        .filter_map(|g| g.struck.as_ref().map(|s| (g, s)))
        .collect();
    struck.sort_by_key(|(g, _)| g.given_at);
    struck
        .into_iter()
        .map(|(g, (who, at, why))| {
            let line = format!(
                "given {} {} — entered in error by {}, {}",
                when(&g.given_at, now),
                name(&g.giver),
                name(who),
                when(at, now)
            );
            match why {
                Some(why) => format!("{line} — {why}"),
                None => line,
            }
        })
        .collect()
}

/// A signer's name, else [`SIGNER_NOT_KNOWN`] (2b's rule).
fn name(who: &Option<String>) -> &str {
    who.as_deref().unwrap_or(SIGNER_NOT_KNOWN)
}

/// An instant in the workstation's zone, as the holds and the settle lines read it.
fn when<Tz: TimeZone>(t: &DateTime<Utc>, now: &DateTime<Tz>) -> String
where
    Tz::Offset: Display,
{
    local_instant(&t.to_rfc3339(), now)
}

#[cfg(test)]
#[path = "stat_view_tests.rs"]
mod tests;
