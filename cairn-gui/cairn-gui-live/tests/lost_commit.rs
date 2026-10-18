//! DB-gated, #820 item 3: a registration whose COMMIT is really lost, end to end.
//!
//! Each link of #815 is pinned on its own: `register_patient` marks its commit (root tree,
//! `identity_commit_lost.rs`), `data_error_from` maps the marker to `OutcomeUnknown`
//! (`src/error.rs`), `TokenStore::settle` spends the search (`cairn-gui-funnel`'s `token.rs`).
//! This drives one real lost COMMIT through all three at once, so a link that stops passing the
//! marker along (a chain rendered to a string, a new arm in `data_error_from`) fails here.
//!
//! The commit is lost the way the root tree's `signoff_interrupted.rs` loses one: a test-only
//! DEFERRED constraint trigger on `patient_registration` makes the backend terminate itself
//! while the transaction COMMITs — a real FATAL answer to a real COMMIT. It is gated on a
//! SESSION setting, so it is inert for every other connection even if a panic leaked it.
mod common;

use cairn_gui_data::port::{DataError, PatientRegistration};
use cairn_gui_funnel::{bound_for_prompt, Restored, TokenStore};
use cairn_gui_live::LiveData;
use cairn_patient_search::SearchQuery;

const ORIGIN: &str = "gui-live-origin";

const DROP_KILL_SWITCH: &str = "
    DROP TRIGGER IF EXISTS cairn_test_kill_at_commit ON patient_registration;
    DROP FUNCTION IF EXISTS cairn_test_kill_at_commit();";

const CREATE_KILL_SWITCH: &str = "
    CREATE FUNCTION cairn_test_kill_at_commit() RETURNS trigger
    LANGUAGE plpgsql SECURITY DEFINER SET search_path = public, pg_temp AS $$
    BEGIN
        IF coalesce(current_setting('cairn.test_kill_at_commit', true), '') <> 'on' THEN
            RETURN NULL;  -- not this session's test: inert
        END IF;
        PERFORM pg_terminate_backend(pg_backend_pid());
        RETURN NULL;
    END $$;
    CREATE CONSTRAINT TRIGGER cairn_test_kill_at_commit
        AFTER INSERT ON patient_registration
        DEFERRABLE INITIALLY DEFERRED
        FOR EACH ROW EXECUTE FUNCTION cairn_test_kill_at_commit();";

/// The clerk presses Register and the commit gets no ordinary answer. The window must learn the
/// chart MAY exist (`OutcomeUnknown`), and the search must be SPENT (`Consumed`), so the next
/// press searches first and finds the chart if it landed, never registering the person twice.
#[tokio::test]
async fn a_lost_registration_commit_spends_the_search() {
    let Some(cs) = common::cs() else { return };
    let (reader, _guard) = common::connect(&cs).await;
    let (sk, _kid) = common::setup(&reader).await;
    reader.batch_execute(DROP_KILL_SWITCH).await.unwrap();
    reader.batch_execute(CREATE_KILL_SWITCH).await.unwrap();

    let victim = common::connect_for_live(&cs).await;
    victim
        .batch_execute("SET cairn.test_kill_at_commit = 'on'")
        .await
        .unwrap();
    let live = LiveData::new(victim, sk, &common::identity(ORIGIN));

    let mut store = TokenStore::new();
    let token = store
        .record(
            SearchQuery::new("Lost Commit", Some("1970-02-03"), &[]),
            bound_for_prompt(&common::nothing_found()),
        )
        .expect("a non-empty query mints a token");
    let attested = store.take(token).expect("the token is redeemable");

    let outcome = live.register(attested, Some("Lost Commit")).await;
    reader.batch_execute(DROP_KILL_SWITCH).await.unwrap();

    match store.settle(outcome) {
        Err((DataError::OutcomeUnknown(_), Restored::Consumed)) => {}
        Err((error, restored)) => panic!(
            "a commit with no ordinary answer MAY have created the chart, so it must arrive as \
             OutcomeUnknown and spend the search; it arrived as {error:?}, {restored:?}"
        ),
        Ok(id) => panic!("the commit was answered FATAL, yet registration returned {id}"),
    }
    assert!(
        store.take(token).is_err(),
        "the spent search cannot license a second registration"
    );
}
