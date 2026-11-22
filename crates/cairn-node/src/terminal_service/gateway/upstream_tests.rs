use super::{parse, route_verdict, Upstream};
use crate::window_access::HbaRule;

fn rule(n: i32, kind: &str, method: &str) -> HbaRule {
    HbaRule {
        file_name: "pg_hba.conf".into(),
        line_number: n,
        kind: kind.into(),
        auth_method: method.into(),
        options: None,
        error: None,
    }
}

/// Debian's stock rules: the Unix socket is peer-authenticated, TCP loopback is SCRAM.
fn debian() -> Vec<HbaRule> {
    vec![
        rule(1, "local", "peer"),
        rule(2, "local", "peer"),
        rule(3, "host", "scram-sha-256"),
    ]
}

#[test]
fn parse_refuses_a_remote_address_a_relative_path_and_a_wrong_socket_name() {
    assert!(parse("10.1.2.3:5432").is_err());
    assert!(parse("run/postgresql/.s.PGSQL.5432").is_err());
    assert!(parse("/run/postgresql/postgres.sock").is_err());
    assert!(parse("/run/postgresql/.s.PGSQL.").is_err());
    assert!(parse("/run/postgresql/.s.PGSQL.54x2").is_err());
    assert!(parse("localhost").is_err());
}

#[test]
fn parse_accepts_loopback_tcp_and_a_postgres_socket_and_round_trips() {
    for s in [
        "127.0.0.1:5432",
        "[::1]:5432",
        "/run/postgresql/.s.PGSQL.5432",
    ] {
        let u = parse(s).unwrap();
        assert_eq!(u.to_string(), s);
    }
    assert!(matches!(parse("[::1]:5432").unwrap(), Upstream::Tcp(_)));
    assert!(matches!(
        parse("/tmp/.s.PGSQL.5433").unwrap(),
        Upstream::Unix(_)
    ));
}

#[test]
fn stock_debian_rules_refuse_the_socket_naming_the_peer_lines_and_accept_tcp() {
    let e = route_verdict(&debian(), &parse("/tmp/.s.PGSQL.5432").unwrap()).unwrap_err();
    assert!(
        e.contains("pg_hba.conf:1") && e.contains("pg_hba.conf:2"),
        "{e}"
    );
    assert!(!e.contains("pg_hba.conf:3"), "{e}");
    assert!(e.contains("reload"), "{e}");
    assert!(route_verdict(&debian(), &parse("127.0.0.1:5432").unwrap()).is_ok());
}

#[test]
fn an_ident_gss_or_sspi_host_line_refuses_tcp() {
    for m in ["ident", "gss", "sspi"] {
        for kind in ["host", "hostnossl", "hostgssenc", "hostssl", "hostnogssenc"] {
            let rules = [rule(9, kind, m), rule(10, "host", "scram-sha-256")];
            let e = route_verdict(&rules, &parse("127.0.0.1:5432").unwrap()).unwrap_err();
            assert!(
                e.contains("pg_hba.conf:9") && e.contains("reload"),
                "{kind} {m}: {e}"
            );
        }
    }
    // The same line is irrelevant to a socket route.
    let rules = [rule(9, "host", "ident"), rule(1, "local", "scram-sha-256")];
    assert!(route_verdict(&rules, &parse("/tmp/.s.PGSQL.5432").unwrap()).is_ok());
}

#[test]
fn md5_and_password_warn_on_the_chosen_route_and_trust_still_refuses() {
    let w = route_verdict(
        &[rule(4, "host", "md5"), rule(5, "host", "password")],
        &parse("[::1]:5432").unwrap(),
    )
    .unwrap();
    assert_eq!(w.len(), 2, "{w:?}");
    assert!(w[0].contains("pg_hba.conf:4"));
    let e = route_verdict(
        &[rule(6, "host", "trust")],
        &parse("127.0.0.1:5432").unwrap(),
    )
    .unwrap_err();
    assert!(e.contains("line 6"), "{e}");
}

#[test]
fn parse_refuses_dot_dot_segments_and_a_trailing_slash() {
    assert!(parse("/x/../y/.s.PGSQL.5432").is_err());
    assert!(parse("/run/postgresql/.s.PGSQL.5432/").is_err());
}

#[test]
fn parse_refuses_mapped_wildcard_and_named_hosts() {
    for s in [
        "[::ffff:10.0.0.5]:5432",
        "[::ffff:127.0.0.1]:5432",
        "0.0.0.0:5432",
        "localhost:5432",
    ] {
        assert!(parse(s).is_err(), "{s}");
    }
}

#[test]
fn a_socket_route_refuses_ident_cert_and_mapped_peer_and_warns_on_md5() {
    let sock = parse("/tmp/.s.PGSQL.5432").unwrap();
    for m in ["ident", "cert"] {
        let e = route_verdict(&[rule(8, "local", m)], &sock).unwrap_err();
        assert!(e.contains("pg_hba.conf:8"), "{m}: {e}");
    }
    let mut mapped = rule(8, "local", "peer");
    mapped.options = Some(vec!["map=x".into()]);
    assert!(route_verdict(&[mapped], &sock)
        .unwrap_err()
        .contains("pg_hba.conf:8"));
    let w = route_verdict(&[rule(4, "local", "md5")], &sock).unwrap();
    assert_eq!(w.len(), 1, "{w:?}");
}

#[test]
fn a_local_peer_line_does_not_affect_a_tcp_route() {
    let w = route_verdict(
        &[rule(1, "local", "peer")],
        &parse("127.0.0.1:5432").unwrap(),
    );
    assert!(w.is_ok());
}

/// The TCP branch is an allow-list like the socket's: a method that is neither SCRAM nor a hash
/// (ldap, pam, radius, bsd) still makes the window's password the credential, but Postgres
/// receives it in clear on the loopback leg, so it warns rather than passing in silence (SF6).
#[test]
fn any_other_tcp_method_warns_that_the_password_is_not_scram_end_to_end() {
    let tcp = parse("127.0.0.1:5432").unwrap();
    for m in ["ldap", "pam", "radius", "bsd"] {
        let w = route_verdict(&[rule(7, "host", m)], &tcp).unwrap();
        assert_eq!(w.len(), 1, "{m}: {w:?}");
        assert!(w[0].contains("pg_hba.conf:7"), "{m}: {w:?}");
        assert!(
            w[0].contains(m) && w[0].contains("scram-sha-256"),
            "{m}: {w:?}"
        );
    }
    let quiet = [rule(7, "host", "scram-sha-256"), rule(8, "host", "reject")];
    assert!(route_verdict(&quiet, &tcp).unwrap().is_empty());
}
