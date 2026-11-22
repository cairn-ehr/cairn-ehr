//! Who may pass the gateway (#909 slice B, D2 step 3): the startup verdict.
//!
//! The gateway reads a remote window's startup packet and must decide, before opening any
//! upstream connection, whether to forward it. Only a window login may pass: never the terminal
//! service's own login (it can write registration rows a window cannot) and never an
//! over-powered role. The decision is split in two so the judgement needs no database:
//! [`read_login_facts`] reads what the login IS (one DB read), [`startup_verdict`] judges it
//! (pure). The window itself is always told the same fixed message; the `Err` text of the verdict
//! is for the gateway's log only, because a distinct reply would let a probe learn which role
//! names exist.
//!
//! [`judge`] wraps the two for the gateway: a facts read that could not be made (the gateway's
//! own database connection is down) is a TRANSIENT refusal (57P03, the window retries), never
//! the permanent 28000, so an outage of the gateway's connection never reads as "you may not".
use super::wire::Startup;
use crate::conn_posture::{over_powered, read_posture_of, PostureFacts};
use crate::terminal_service::commit::primary_message;
use anyhow::{bail, Context};
use tokio_postgres::Client;

/// What the window is told, whatever the reason (28000 `invalid_authorization_specification`).
pub const REFUSED: &str = "this login may not connect through the node's terminal gateway";

/// The facts [`startup_verdict`] judges about the login a startup names.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct LoginFacts {
    /// `pg_roles.rolcanlogin`: a group role is not a login.
    pub can_login: bool,
    /// Member of `cairn_window` (directly or through another role).
    pub window_member: bool,
    /// Member of `cairn_terminal_service`. That role inherits `cairn_window`, so a service member
    /// is a window member too; the verdict must refuse it anyway.
    pub service_member: bool,
    /// What the login can do that a window must not (shared with the window's own launch check).
    pub posture: PostureFacts,
}

/// Read [`LoginFacts`] for the role a startup names. `user` goes in as a bind parameter, never
/// into the SQL text, so a hostile name cannot inject anything. A role that does not exist is an
/// `Err` (checked first: `pg_has_role` on an unknown name would also error, but less legibly);
/// the caller refuses on any `Err`.
pub async fn read_login_facts(db: &Client, user: &str) -> anyhow::Result<LoginFacts> {
    // shortcut: an unknown role returns after one query, a known one after two, so existence leaks
    // by timing to an already mutually-authenticated terminal; upgrade if the gateway ever faces
    // unauthenticated peers at this step.
    let row = db
        .query_opt(
            "SELECT rolcanlogin, \
                    pg_has_role($1::text::name, 'cairn_window', 'MEMBER'), \
                    pg_has_role($1::text::name, 'cairn_terminal_service', 'MEMBER') \
               FROM pg_roles WHERE rolname = $1::text::name",
            &[&user],
        )
        .await
        .context("reading whether this login belongs to the window or the service")?;
    let Some(row) = row else {
        bail!("the startup names a role that does not exist");
    };
    Ok(LoginFacts {
        can_login: row.get(0),
        window_member: row.get(1),
        service_member: row.get(2),
        posture: read_posture_of(db, user).await?,
    })
}

/// Pure: may this startup be forwarded to Postgres? `Err` carries the reason for the log.
pub fn startup_verdict(s: &Startup, node_db: &str, facts: &LoginFacts) -> Result<(), &'static str> {
    if s.get("user").is_none() {
        return Err("the startup names no user");
    }
    match s.get("database") {
        None => return Err("the startup names no database"),
        Some(d) if d != node_db => return Err("the startup names another database"),
        Some(_) => {}
    }
    // ALLOW-LIST, not a deny-list: Postgres applies every startup key other than user, database,
    // options, replication and `_pq_.*` as a GUC at connect (case-insensitive), so a bare
    // `role=…` does what `options=-c role=…` does, after we vetted the login. Only what the
    // window's driver sends passes; `replication` (the plan's named refusal), `options` and
    // anything else refuse.
    const ALLOWED_KEYS: [&str; 4] = ["user", "database", "client_encoding", "application_name"];
    if s.params
        .iter()
        .any(|(k, _)| !ALLOWED_KEYS.contains(&k.as_str()))
    {
        return Err("the startup carries a parameter a window never sends");
    }
    if !facts.can_login {
        return Err("the role is not a login");
    }
    if facts.service_member {
        return Err("the role is the terminal service's, not a window's");
    }
    if !facts.window_member {
        return Err("the login is not a window member");
    }
    if let Some(why) = over_powered(&facts.posture) {
        return Err(why);
    }
    Ok(())
}

/// What a transient refusal tells the window (57P03 cannot_connect_now: it retries).
pub const NOT_NOW: &str =
    "the node's terminal gateway cannot check logins right now; try again shortly";

/// Why a startup is not forwarded: what the window is told, and why (for the log only).
pub struct Refusal {
    pub sqlstate: &'static str,
    pub message: &'static str,
    pub why: String,
}

/// Marks an error as "the gateway could not ask", not "the answer is no".
#[derive(Debug)]
pub struct DbUnavailable;

impl std::fmt::Display for DbUnavailable {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("the gateway's database connection is unavailable")
    }
}

/// **Pure.** A facts read that failed in the database (or never reached it) is transient: 57P03.
/// One that failed on its own verdict (the role does not exist) and every verdict refusal is
/// 28000. Either way nothing is forwarded.
pub fn judge(facts: anyhow::Result<LoginFacts>, s: &Startup, node_db: &str) -> Result<(), Refusal> {
    let refused = |why: String| Refusal {
        sqlstate: "28000",
        message: REFUSED,
        why,
    };
    match facts {
        Ok(f) => startup_verdict(s, node_db, &f).map_err(|why| refused(why.to_string())),
        Err(e)
            if e.downcast_ref::<DbUnavailable>().is_some()
                || e.chain().any(|c| c.is::<tokio_postgres::Error>()) =>
        {
            Err(Refusal {
                sqlstate: "57P03",
                message: NOT_NOW,
                why: primary_message(&e),
            })
        }
        Err(e) => Err(refused(primary_message(&e))),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn startup(params: &[(&str, &str)]) -> Startup {
        Startup {
            raw: Vec::new(),
            params: params
                .iter()
                .map(|(k, v)| (k.to_string(), v.to_string()))
                .collect(),
        }
    }

    fn ok_startup() -> Startup {
        startup(&[("user", "w"), ("database", "node")])
    }

    fn no_posture() -> PostureFacts {
        PostureFacts {
            superuser: false,
            owns_cairn_table: false,
            shapes_schema: false,
            reads_vault: false,
            inserts_event_log: false,
            runs_peer_door: false,
        }
    }

    fn window() -> LoginFacts {
        LoginFacts {
            can_login: true,
            window_member: true,
            service_member: false,
            posture: no_posture(),
        }
    }

    #[test]
    fn a_window_login_passes() {
        assert_eq!(startup_verdict(&ok_startup(), "node", &window()), Ok(()));
    }

    #[test]
    fn startup_shape_refusals() {
        let f = window();
        let v = |s: Startup| startup_verdict(&s, "node", &f);
        assert!(v(startup(&[("database", "node")])).is_err(), "no user");
        assert!(v(startup(&[("user", "w")])).is_err(), "no database");
        assert!(v(startup(&[("user", "w"), ("database", "other")])).is_err());
        // Postgres reads any other startup key as a GUC set at connect, so all of these refuse.
        for extra in [
            "replication",
            "options",
            "role",
            "ROLE",
            // Postgres matches `user` and `database` case-sensitively: these are GUCs (T4).
            "User",
            "Database",
            "search_path",
            "_pq_.foo",
        ] {
            let s = startup(&[("user", "w"), ("database", "node"), (extra, "x")]);
            assert!(v(s).is_err(), "{extra} must be refused");
        }
        // Any value counts, including an empty one or "false".
        let s = startup(&[
            ("user", "w"),
            ("database", "node"),
            ("replication", "false"),
        ]);
        assert!(v(s).is_err());
    }

    #[test]
    fn login_refusals() {
        let s = ok_startup();
        let mut f = window();
        f.can_login = false;
        assert!(startup_verdict(&s, "node", &f).is_err(), "not a login");
        let mut f = window();
        f.window_member = false;
        assert!(startup_verdict(&s, "node", &f).is_err(), "not a member");
        // The real service is a window member too; it must still be refused.
        let mut f = window();
        f.service_member = true;
        assert!(startup_verdict(&s, "node", &f).is_err(), "service member");
        f.window_member = false;
        assert!(startup_verdict(&s, "node", &f).is_err(), "service only");
    }

    #[test]
    fn each_posture_fact_refuses() {
        let s = ok_startup();
        let set: [fn(&mut PostureFacts); 6] = [
            |p| p.superuser = true,
            |p| p.owns_cairn_table = true,
            |p| p.shapes_schema = true,
            |p| p.reads_vault = true,
            |p| p.inserts_event_log = true,
            |p| p.runs_peer_door = true,
        ];
        for apply in set {
            let mut f = window();
            apply(&mut f.posture);
            assert!(startup_verdict(&s, "node", &f).is_err());
        }
    }

    fn bare_startup() -> Startup {
        Startup {
            raw: Vec::new(),
            params: vec![
                ("user".into(), "w".into()),
                ("database".into(), "node".into()),
            ],
        }
    }

    #[test]
    fn a_check_that_could_not_be_made_is_transient_and_a_no_is_28000() {
        let down = Err(anyhow::anyhow!("closed")).context(DbUnavailable);
        assert_eq!(
            judge(down, &bare_startup(), "node").unwrap_err().sqlstate,
            "57P03"
        );
        let unknown = Err(anyhow::anyhow!(
            "the startup names a role that does not exist"
        ));
        let r = judge(unknown, &bare_startup(), "node").unwrap_err();
        assert_eq!((r.sqlstate, r.message), ("28000", REFUSED));
        let service = LoginFacts {
            can_login: true,
            window_member: true,
            service_member: true,
            posture: no_posture(),
        };
        assert_eq!(
            judge(Ok(service), &bare_startup(), "node")
                .unwrap_err()
                .sqlstate,
            "28000"
        );
    }
}
