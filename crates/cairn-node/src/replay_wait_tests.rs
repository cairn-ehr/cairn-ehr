use super::*;

#[test]
fn only_55p03_is_a_lock_timeout() {
    assert!(is_lock_timeout(Some("55P03")));
    // A deadlock, a statement timeout and a plain failure are not waits to sit out.
    for other in ["40P01", "57014", "42883", "25P02"] {
        assert!(!is_lock_timeout(Some(other)), "{other}");
    }
    // No SQLSTATE at all (a dropped socket) is not one either.
    assert!(!is_lock_timeout(None));
}

#[test]
fn backoff_doubles_then_caps() {
    let ms = |a| backoff(a).as_millis();
    assert_eq!(ms(0), 250);
    assert_eq!(ms(1), 500);
    assert_eq!(ms(2), 1000);
    assert_eq!(ms(3), 2000);
    assert_eq!(ms(4), 4000);
    for attempt in [5, 6, 31, 32, u32::MAX] {
        assert_eq!(ms(attempt), 5000, "attempt {attempt}");
    }
}

/// Final review M5: the stamp's INSERT runs while `lock_timeout` is still armed, so a `55P03`
/// there must be waited out like every other replay step — not fail a replay that already
/// landed. A source guard: no test can time a blocker between the heal and the stamp.
#[test]
fn the_stamp_waits_out_a_lock_timeout_too() {
    let db = include_str!("db.rs");
    let stamp = db
        .find("INSERT INTO node_schema (version, loader_build, schema_hash,")
        .expect("the stamp's INSERT");
    let retry = db[..stamp]
        .rfind("crate::replay_wait::retry_on_lock_timeout(&client, \"the schema stamp\", || {")
        .expect("the stamp runs through the retry loop");
    // Nothing but the call itself between the retry and the INSERT.
    let between = &db[retry..stamp];
    assert!(!between.contains(';'), "{between}");
    assert!(stamp < db.find("crate::replay_wait::disarm(&client)").unwrap());
}
