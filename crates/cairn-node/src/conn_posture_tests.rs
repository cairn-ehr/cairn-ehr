use super::{over_powered, require_local, window_schema_verdict, PostureFacts};
use crate::db::{embedded_schema_hash, embedded_schema_version, SchemaStamp, LOADER_BUILD};

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

fn stamp(version: i32, loader: &str, hash: Option<Vec<u8>>) -> SchemaStamp {
    SchemaStamp {
        version,
        loader_build: loader.into(),
        schema_hash: hash,
    }
}

#[test]
fn a_matching_stamp_lets_the_window_start() {
    let s = stamp(
        embedded_schema_version(),
        LOADER_BUILD,
        Some(embedded_schema_hash().to_vec()),
    );
    assert!(window_schema_verdict(Some(&s)).is_ok());
}

#[test]
fn a_newer_database_says_the_window_is_older() {
    let s = stamp(
        embedded_schema_version() + 1,
        LOADER_BUILD,
        Some(embedded_schema_hash().to_vec()),
    );
    assert!(window_schema_verdict(Some(&s))
        .unwrap_err()
        .contains("older than"));
}

#[test]
fn every_other_mismatch_sends_the_operator_to_provision_window_role() {
    let v = embedded_schema_version();
    let h = embedded_schema_hash().to_vec();
    let cases = [
        None,
        Some(stamp(v - 1, LOADER_BUILD, Some(h.clone()))),
        Some(stamp(v, LOADER_BUILD, None)),
        Some(stamp(v, "cairn-sync 0.0.0", Some(h))),
    ];
    for c in cases {
        let e = window_schema_verdict(c.as_ref()).unwrap_err();
        assert!(e.contains("provision-window-role"));
        assert!(!e.contains("older than"));
    }
}

/// At the SAME generation the database was prepared by a different build (another hash or
/// loader): only the cairn-node built with this window can stamp what it expects, so say so.
#[test]
fn a_same_generation_mismatch_names_the_cairn_node_built_with_this_window() {
    let v = embedded_schema_version();
    let h = embedded_schema_hash().to_vec();
    for s in [
        stamp(v, LOADER_BUILD, None),
        stamp(v, LOADER_BUILD, Some(vec![0; h.len()])),
        stamp(v, "cairn-sync 0.0.0", Some(h.clone())),
    ] {
        let e = window_schema_verdict(Some(&s)).unwrap_err();
        assert!(e.contains("the cairn-node built with this window"), "{e}");
    }
    let older = window_schema_verdict(Some(&stamp(v - 1, LOADER_BUILD, Some(h)))).unwrap_err();
    assert!(!older.contains("built with this window"), "{older}");
}
