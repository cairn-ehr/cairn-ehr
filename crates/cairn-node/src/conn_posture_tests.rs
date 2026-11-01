use super::{over_powered, require_local, PostureFacts};

#[test]
fn local_forms_pass() {
    for c in [
        "host=/tmp dbname=x",
        "host=localhost",
        "host=127.0.0.1",
        "host=127.4.5.6",
        "host=::1",
        "dbname=x",
    ] {
        assert!(require_local(c).is_ok(), "{c} should be local");
    }
}

#[test]
fn remote_forms_refuse() {
    for (c, host) in [
        ("host=10.0.0.3", "10.0.0.3"),
        ("host=db.example", "db.example"),
        ("host=localhost,10.0.0.3", "10.0.0.3"),
    ] {
        let e = require_local(c).unwrap_err().to_string();
        assert!(e.contains(host) && e.contains("another machine"));
    }
}

#[test]
fn hostaddr_wins() {
    assert!(require_local("host=localhost hostaddr=10.0.0.3").is_err());
    assert!(require_local("host=db.example hostaddr=127.0.0.1").is_ok());
}

#[test]
fn password_never_echoed() {
    let e = require_local("host=10.0.0.3 password=s3cret").unwrap_err();
    assert!(!e.to_string().contains("s3cret"));
}

#[test]
fn unparsable_is_refused() {
    assert!(require_local("port=notanumber").is_err());
}

/// The window's own login: none of the five powers.
fn window() -> PostureFacts {
    PostureFacts {
        superuser: false,
        owns_cairn_table: false,
        reads_vault: false,
        inserts_event_log: false,
        runs_peer_door: false,
    }
}

#[test]
fn a_window_connection_is_not_over_powered() {
    assert_eq!(over_powered(&window()), None);
}

#[test]
fn each_power_alone_is_over_powered() {
    let each: [fn(&mut PostureFacts); 5] = [
        |f| f.superuser = true,
        |f| f.owns_cairn_table = true,
        |f| f.reads_vault = true,
        |f| f.inserts_event_log = true,
        |f| f.runs_peer_door = true,
    ];
    for (i, set) in each.iter().enumerate() {
        let mut f = window();
        set(&mut f);
        let why = over_powered(&f).unwrap_or_else(|| panic!("power {i} alone was let through"));
        assert!(why.ends_with('.') && !why.is_empty(), "{why}");
    }
}
