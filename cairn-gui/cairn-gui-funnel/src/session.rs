//! One window's registration form, as the funnel sees it: the search it may attest to, the
//! exact name that search ran on, and which searches have gone stale.
//!
//! # Why this exists on top of [`TokenStore`]
//!
//! [`TokenStore`] pairs a query with the list it produced. It does not carry the NAME, and it
//! cannot: `SearchQuery` holds lowercased, deduplicated tokens, while `register_patient` must be
//! handed the raw typed string the query was built from (its own doc says nothing in its types
//! enforces this). The design's answer is *"one typed string feeds `SearchQuery::new` and
//! `register` alike"* — true of the webview by construction, since the form has one name field.
//! This module makes it true of the Rust side too: the name is stored WITH the token when the
//! search is recorded, and handed back with the attestation when it is taken, so a caller has
//! no second name to pass.
//!
//! # Why revisions, and why trusting the webview's number is safe
//!
//! The step-3 search re-runs in the background as the clerk types, and two searches in flight
//! can finish in either order. [`TokenStore::record`] replaces whatever it holds, so without a
//! freshness check a search for the form as it was a keystroke ago could land LAST and become
//! the one a click on Register attests. The webview therefore numbers every edit (a
//! `revision`), announces each edit with [`FunnelSession::edited`], and sends the number with
//! each search; this module refuses to record a search older than the newest revision it has
//! seen.
//!
//! The number comes from the webview, and that is safe: a wrong revision can only make a search
//! be DROPPED (the clerk waits for the next one) or be ACCEPTED — and an accepted search is one
//! that genuinely ran, on exactly the name stored beside it. No revision value can pair a name
//! with a query built from a different one, because both come from the same [`FormSnapshot`].

use crate::prompt::PromptList;
use crate::token::{AttestedSearch, Restored, SearchToken, TokenError, TokenStore};
use cairn_patient_search::SearchQuery;
use serde::Deserialize;

/// The registration form as the webview sent it, at one revision.
#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
pub struct FormSnapshot {
    /// The webview's edit counter when the form was read. Increases with every edit.
    pub revision: u64,
    /// The name field exactly as typed — never trimmed here, never reassembled.
    pub raw_name: String,
    /// The date-of-birth field exactly as typed; blank means "not supplied" (principle 4:
    /// an unknown date of birth is a first-class answer, not a missing required field).
    pub birth_date: String,
}

impl FormSnapshot {
    /// The ONE place a funnel query is built from a form.
    ///
    /// The search and the record both call this on the same snapshot, so what was searched
    /// and what is attested cannot differ. Blank-after-trim birth date is "not supplied", the
    /// rule `register_patient` also applies.
    pub fn query(&self) -> SearchQuery {
        let dob = Some(self.birth_date.trim()).filter(|d| !d.is_empty());
        SearchQuery::new(&self.raw_name, dob, &[])
    }
}

/// What [`FunnelSession::record`] did with a search.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Recorded {
    /// Recorded; this token redeems it.
    Current(SearchToken),
    /// A newer revision of the form had already been seen, so this search described a form
    /// that no longer exists. NOT recorded; nothing it found can be attested.
    Stale,
}

/// Custody of one window's registration search, and the name it ran on.
#[derive(Debug, Default)]
pub struct FunnelSession {
    store: TokenStore,
    /// The raw name the held search ran on, beside the token that names that search. Set and
    /// cleared only alongside the store's own held search.
    name_for: Option<(SearchToken, String)>,
    /// The highest form revision seen by `edited` or `record`.
    revision: u64,
}

impl FunnelSession {
    pub fn new() -> Self {
        Self::default()
    }

    /// The clerk edited the form: whatever is held no longer describes it.
    ///
    /// Discards unconditionally, even for a revision older than one already seen (an edit
    /// notice arriving out of order). Discarding is the fail-safe direction — the worst it
    /// costs is one re-search — while the revision floor only ever rises.
    pub fn edited(&mut self, revision: u64) {
        self.revision = self.revision.max(revision);
        self.store.discard();
        self.name_for = None;
    }

    /// Record the search that ran on `form`, unless a newer revision was already seen.
    ///
    /// An equal revision is accepted: an edit is announced first and the search for the
    /// edited form then carries the same number. An unsearchable form is refused exactly as
    /// [`TokenStore::record`] refuses it, and — as there — invalidates what was held.
    pub fn record(
        &mut self,
        form: &FormSnapshot,
        prompt: PromptList,
    ) -> Result<Recorded, TokenError> {
        if form.revision < self.revision {
            return Ok(Recorded::Stale);
        }
        self.revision = form.revision;
        // Cleared BEFORE the `?`: a refused record has invalidated the store too, so the old
        // name must not outlive the search it belonged to.
        self.name_for = None;
        let token = self.store.record(form.query(), prompt)?;
        self.name_for = Some((token, form.raw_name.clone()));
        Ok(Recorded::Current(token))
    }

    /// Take the search `token` names for a registration, WITH the raw name it ran on.
    ///
    /// Every successful take must reach [`FunnelSession::settle`] — the same two-ended
    /// contract [`TokenStore::take`] has.
    pub fn take_for_register(
        &mut self,
        token: SearchToken,
    ) -> Result<(AttestedSearch, String), TokenError> {
        let attested = self.store.take(token)?;
        match &self.name_for {
            Some((held, name)) if *held == token => Ok((attested, name.clone())),
            // Unreachable while `record` is the only writer of both halves. If it is ever
            // reached, put the search back and refuse rather than register a chart with no
            // name — the refusal's remedy ("let the search run again") is always safe.
            _ => {
                let _ = self.store.restore(attested);
                Err(TokenError::Absent)
            }
        }
    }

    /// Settle a registration's outcome — [`TokenStore::settle`], plus forgetting the name once
    /// a success has consumed the form.
    ///
    /// On failure the name is KEPT: a restored search is redeemable again and must still carry
    /// the name it ran on (the design's *"Register fails. The form keeps its values."*).
    pub fn settle<T, E>(
        &mut self,
        outcome: Result<T, (E, AttestedSearch)>,
    ) -> Result<T, (E, Restored)> {
        let settled = self.store.settle(outcome);
        if settled.is_ok() {
            self.name_for = None;
        }
        settled
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::prompt::bound_for_prompt;
    use cairn_patient_search::{Candidate, CandidateList, TrustState};
    use uuid::Uuid;

    fn candidate(n: u128) -> Candidate {
        Candidate {
            patient_id: Uuid::from_u128(n),
            display_name: format!("Patient {n}"),
            age: None,
            trust: TrustState::Confirmed,
            last_activity: None,
            locale: None,
            photo_ref: None,
        }
    }

    fn form(revision: u64, name: &str, dob: &str) -> FormSnapshot {
        FormSnapshot {
            revision,
            raw_name: name.to_string(),
            birth_date: dob.to_string(),
        }
    }

    /// A one-candidate prompt, bounded the only way `record` accepts.
    fn prompt(n: u128) -> PromptList {
        bound_for_prompt(&CandidateList {
            candidates: vec![candidate(n)],
            incomplete: false,
            incomplete_reason: None,
        })
    }

    /// Record at `form` and insist it was current — the common opening of these tests.
    fn current(s: &mut FunnelSession, f: &FormSnapshot, n: u128) -> SearchToken {
        match s.record(f, prompt(n)).expect("a searchable form records") {
            Recorded::Current(t) => t,
            Recorded::Stale => panic!("expected a current search"),
        }
    }

    /// The design's "one typed string feeds `SearchQuery::new` and `register` alike", made a
    /// property of the session: the name handed out for registration is the RAW string the
    /// recorded query was built from — untrimmed, never reassembled from tokens.
    #[test]
    fn the_registered_name_is_the_one_the_search_ran_on() {
        let mut s = FunnelSession::new();
        let f = form(1, "  John   Smith ", "1980-01-01");
        let t = current(&mut s, &f, 1);
        let (attested, name) = s.take_for_register(t).unwrap();
        assert_eq!(name, "  John   Smith ");
        assert_eq!(attested.query(), &f.query());
    }

    /// Review Focus 1: "Jon Smith" is searched, corrected to "John Smith" and searched again,
    /// and the "Jon" search lands LAST. It must not replace the newer one.
    #[test]
    fn a_search_for_an_older_form_revision_is_dropped() {
        let mut s = FunnelSession::new();
        let newer = current(&mut s, &form(2, "John Smith", "1980-01-01"), 2);
        assert_eq!(
            s.record(&form(1, "Jon Smith", "1980-01-01"), prompt(1))
                .unwrap(),
            Recorded::Stale
        );
        let (attested, name) = s.take_for_register(newer).unwrap();
        assert_eq!(name, "John Smith");
        assert_eq!(
            attested.displayed().candidates[0].patient_id,
            Uuid::from_u128(2)
        );
    }

    /// An edit announced before the search for the pre-edit form lands: that search is stale,
    /// while the search for the edited form itself (same revision) is current.
    #[test]
    fn a_search_for_the_form_before_an_edit_is_dropped() {
        let mut s = FunnelSession::new();
        s.edited(5);
        assert_eq!(
            s.record(&form(4, "Jon", "1980"), prompt(1)).unwrap(),
            Recorded::Stale
        );
        assert!(matches!(
            s.record(&form(5, "John", "1980"), prompt(1)).unwrap(),
            Recorded::Current(_)
        ));
    }

    #[test]
    fn an_edit_makes_the_held_search_unredeemable() {
        let mut s = FunnelSession::new();
        let t = current(&mut s, &form(1, "John Smith", "1980"), 1);
        s.edited(2);
        assert_eq!(s.take_for_register(t).unwrap_err(), TokenError::Absent);
    }

    /// An out-of-order (older) edit notice still discards: discarding is the fail-safe
    /// direction, and it must not lower the revision floor.
    #[test]
    fn an_older_edit_notice_still_discards_but_keeps_the_floor() {
        let mut s = FunnelSession::new();
        let t = current(&mut s, &form(3, "John Smith", "1980"), 1);
        s.edited(2);
        assert_eq!(s.take_for_register(t).unwrap_err(), TokenError::Absent);
        assert_eq!(
            s.record(&form(2, "John Smith", "1980"), prompt(1)).unwrap(),
            Recorded::Stale,
            "revision 3 was already seen, so a revision-2 search is still stale"
        );
    }

    #[test]
    fn a_failed_registration_keeps_the_name_with_the_search() {
        let mut s = FunnelSession::new();
        let t = current(&mut s, &form(1, "John Smith", "1980"), 1);
        let (attested, _) = s.take_for_register(t).unwrap();
        let out: Result<(), (&str, Restored)> = s.settle(Err(("db down", attested)));
        assert_eq!(out.unwrap_err().1, Restored::Kept);
        let (_, name) = s
            .take_for_register(t)
            .expect("a Kept search is redeemable again");
        assert_eq!(name, "John Smith");
    }

    #[test]
    fn a_successful_registration_consumes_the_form() {
        let mut s = FunnelSession::new();
        let t = current(&mut s, &form(1, "John Smith", "1980"), 1);
        let (_attested, _) = s.take_for_register(t).unwrap();
        // The port consumed the attestation on success, so `Ok` carries no value back.
        let ok: Result<u8, (&str, Restored)> = s.settle(Ok(7));
        assert_eq!(ok.unwrap(), 7);
        assert_eq!(s.take_for_register(t).unwrap_err(), TokenError::Absent);
    }

    #[test]
    fn an_unsearchable_form_records_nothing_and_clears_the_name() {
        let mut s = FunnelSession::new();
        let t = current(&mut s, &form(1, "John Smith", "1980"), 1);
        assert_eq!(
            s.record(&form(2, "  ", ""), prompt(1)).unwrap_err(),
            TokenError::EmptyQuery
        );
        assert_eq!(s.take_for_register(t).unwrap_err(), TokenError::Absent);
    }

    #[test]
    fn the_query_treats_a_blank_birth_date_as_not_supplied() {
        assert_eq!(form(1, "John", "   ").query().birth_date, None);
        assert_eq!(
            form(1, "John", " 1980 ").query().birth_date.as_deref(),
            Some("1980")
        );
    }

    /// The webview sends the form as JSON; the field names are the contract.
    #[test]
    fn a_form_snapshot_deserializes_from_the_webviews_json() {
        let f: FormSnapshot =
            serde_json::from_str(r#"{"revision":3,"raw_name":"Ada","birth_date":""}"#).unwrap();
        assert_eq!(f.revision, 3);
        assert_eq!(f.raw_name, "Ada");
    }
}
