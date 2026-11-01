use super::{hba_verdict, HbaRule};

fn rule(n: i32, kind: &str, method: &str, options: Option<&[&str]>) -> HbaRule {
    HbaRule {
        line_number: n,
        kind: kind.into(),
        auth_method: method.into(),
        options: options.map(|o| o.iter().map(|s| s.to_string()).collect()),
        error: None,
    }
}

#[test]
fn credentialed_methods_pass_clean() {
    let rules = [
        rule(1, "host", "scram-sha-256", None),
        rule(2, "host", "md5", None),
        rule(3, "host", "reject", None),
        rule(4, "hostssl", "cert", None),
    ];
    assert_eq!(hba_verdict(&rules).unwrap(), Vec::<String>::new());
}

#[test]
fn one_trust_line_is_refused_by_line_number() {
    let e = hba_verdict(&[
        rule(1, "host", "scram-sha-256", None),
        rule(7, "local", "trust", None),
    ])
    .unwrap_err();
    assert!(e.contains("line 7") && e.contains("'local'") && e.contains("trust"));
    assert!(!e.contains("line 1:"));
}

#[test]
fn every_trust_line_is_named() {
    let e = hba_verdict(&[
        rule(3, "host", "trust", None),
        rule(9, "host", "trust", None),
    ])
    .unwrap_err();
    assert!(e.contains("line 3") && e.contains("line 9"));
}

#[test]
fn an_unreadable_rule_is_refused() {
    let mut r = rule(4, "", "", None);
    r.error = Some("bad".into());
    assert!(hba_verdict(&[r]).unwrap_err().contains("line 4"));
}

#[test]
fn peer_without_map_warns_and_with_map_does_not() {
    assert_eq!(
        hba_verdict(&[rule(5, "local", "peer", None)])
            .unwrap()
            .len(),
        1
    );
    assert!(hba_verdict(&[rule(5, "local", "peer", Some(&["map=x"]))])
        .unwrap()
        .is_empty());
}
