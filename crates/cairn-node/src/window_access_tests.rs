use super::{hba_verdict, HbaRule};

fn rule(n: i32, kind: &str, method: &str, options: Option<&[&str]>) -> HbaRule {
    HbaRule {
        file_name: "pg_hba.conf".into(),
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
    // Switching a passwordless owner (or the daemon's login) to scram first would lock it out.
    let pw = e.find("give every login").expect("passwords come first");
    assert!(pw < e.find("scram-sha-256").unwrap());
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

#[test]
fn a_stale_hba_file_is_refused() {
    assert!(super::hba_freshness(true)
        .unwrap_err()
        .contains("Reload the server"));
    assert!(super::hba_freshness(false).is_ok());
}

#[test]
fn existing_role_refusals() {
    use super::{existing_role_refusal as r, RoleFacts};
    let ok = RoleFacts {
        can_login: true,
        has_attribute_power: false,
        other_membership: false,
        has_members: false,
    };
    assert!(r(&ok, None).is_none());
    assert!(r(
        &RoleFacts {
            can_login: false,
            ..ok
        },
        None
    )
    .is_some());
    assert!(r(
        &RoleFacts {
            has_attribute_power: true,
            ..ok
        },
        None
    )
    .is_some());
    assert!(r(
        &RoleFacts {
            other_membership: true,
            ..ok
        },
        None
    )
    .is_some());
    // A role others are members of: granting it cairn_window would hand that to all of them.
    assert!(r(
        &RoleFacts {
            has_members: true,
            ..ok
        },
        None
    )
    .unwrap()
    .contains("other roles are members of it"));
    assert!(r(&ok, Some("the login is a database superuser."))
        .unwrap()
        .contains("Choose a new name"));
}

#[test]
fn a_refusal_names_the_included_file_it_is_in() {
    let mut r = rule(2, "host", "trust", None);
    r.file_name = "/etc/postgresql/extra.conf".into();
    let e = hba_verdict(&[r]).unwrap_err();
    assert!(e.contains("/etc/postgresql/extra.conf") && e.contains("line 2"));
}
