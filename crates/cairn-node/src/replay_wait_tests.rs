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
