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
use super::wire::Startup;
use crate::conn_posture::{over_powered, read_posture_of, PostureFacts};
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
    if s.get("replication").is_some() {
        return Err("the startup asks for a replication connection");
    }
    // `options` can set `-c role=…` and other GUCs after this vetting; a window never sends it.
    if s.get("options").is_some() {
        return Err("the startup carries connection options");
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
        for extra in ["replication", "options"] {
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
}
