use super::require_local;

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
