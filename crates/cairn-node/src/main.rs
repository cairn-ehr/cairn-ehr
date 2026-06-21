use clap::{Parser, Subcommand};
use std::net::SocketAddr;
use std::path::PathBuf;

#[derive(Parser)]
#[command(name = "cairn-node", about = "A Cairn federation node")]
struct Cli {
    /// PostgreSQL connection string. `init` needs DDL privileges (it loads the
    /// schema and creates the `cairn_node` role); the RUNTIME commands
    /// (`serve`/`run`/`peers`/…) should connect as an UNPRIVILEGED role so the
    /// in-DB submit/admission gate is unbypassable — create a login role and
    /// `GRANT cairn_node TO <that role>`, then point `--conn`/`CAIRN_CONN` at it.
    /// `status` reports whether the gate actually binds the connected role
    /// (`db_floor ENFORCED` vs `BYPASSABLE`). See `db/007_node_federation.sql`.
    #[arg(long, env = "CAIRN_CONN")] conn: String,
    #[arg(long, default_value = "node.key")] key: PathBuf,
    #[command(subcommand)] cmd: Cmd,
}

#[derive(Subcommand)]
enum Cmd {
    /// Provision this node: mint a keypair and append the genesis enrollment.
    Init { #[arg(long)] name: String, #[arg(long)] address: String },
    /// Print this node's identity (node_id, pubkey, fingerprint, address).
    Identity,
    /// Generate a signed pairing offer (base64) for out-of-band exchange.
    PairOffer {
        #[arg(long, default_value = "cairn")]
        nonce: String,
    },
    /// Accept a peer's pairing offer (base64).  Prints the peer fingerprint and
    /// requires a typed YES confirmation before authoring the peer.added event.
    PairAccept {
        offer: String,
        #[arg(long)] role: Option<String>,
    },
    /// List all peers (active and revoked).
    Peers,
    /// Revoke trust for a peer node.
    Unpeer { node_id: String },
    /// Print this node's honest assembly state (peers, keystore health, DR escrow stub).
    Status,
    /// Serve this node's `node_event` log to pinned-mTLS peers (federation sync).
    Serve {
        #[arg(long, default_value = "0.0.0.0:7843")]
        listen: SocketAddr,
    },
    /// Unattended: serve in the background and pull from `peer` on an interval,
    /// surviving link drops (availability over consistency).
    Run {
        #[arg(long, default_value = "0.0.0.0:7843")]
        listen: SocketAddr,
        #[arg(long)]
        peer: SocketAddr,
        #[arg(long, default_value_t = 5)]
        interval_secs: u64,
    },
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let cli = Cli::parse();
    match cli.cmd {
        Cmd::Init { name, address } => {
            let db = cairn_node::db::connect_and_load_schema(&cli.conn).await?;
            let (sk, kid) = cairn_node::keystore::generate_and_seal(&cli.key, None)?;
            let node_id = cairn_node::identity::provision(&db, &sk, &kid, &name, &address).await?;
            println!("provisioned node {node_id}\nfingerprint {}", cairn_event::short_fingerprint(&kid)?);
        }
        Cmd::Identity => {
            let db = cairn_node::db::connect(&cli.conn).await?;
            let id = cairn_node::identity::load_local(&db).await?;
            println!("node_id     {}\npubkey      {}\nfingerprint {}\naddress     {}",
                id.node_id_hex, id.pubkey_hex, id.fingerprint, id.address);
        }
        Cmd::PairOffer { nonce } => {
            let sk = cairn_node::keystore::load(&cli.key, None)?;
            let db = cairn_node::db::connect(&cli.conn).await?;
            let id = cairn_node::identity::load_local(&db).await?;
            let offer = cairn_node::pairing::make_offer(&id, &sk, &nonce)?;
            println!("{offer}");
        }
        Cmd::PairAccept { offer, role } => {
            let bundle = cairn_node::pairing::read_offer(&offer)?;
            eprintln!(
                "Peer fingerprint: {}\nConfirm it matches what the peer displays, then type YES:",
                bundle.fingerprint
            );
            let mut line = String::new();
            std::io::stdin().read_line(&mut line)?;
            if line.trim() != "YES" {
                anyhow::bail!("pairing aborted: fingerprint not confirmed");
            }
            let sk = cairn_node::keystore::load(&cli.key, None)?;
            let db = cairn_node::db::connect(&cli.conn).await?;
            let id = cairn_node::identity::load_local(&db).await?;
            // Stamp signer_key_id with the key we actually sign with (the keystore),
            // not the DB row; on key/DB drift the door then gives a legible rejection.
            let kid = hex::encode(sk.verifying_key().to_bytes());
            cairn_node::identity::author_peer(
                &db, &sk, &kid, &id.node_id_hex, &bundle, role.as_deref(),
            ).await?;
            println!("peered with {}", bundle.node_id_hex);
        }
        Cmd::Peers => {
            let db = cairn_node::db::connect(&cli.conn).await?;
            let peers = cairn_node::identity::list_peers(&db).await?;
            if peers.is_empty() {
                println!("no peers");
            } else {
                for p in &peers {
                    println!(
                        "{} fp={} role={} status={}",
                        p.peer_node_id_hex,
                        p.fingerprint,
                        p.role.as_deref().unwrap_or("-"),
                        p.status,
                    );
                }
            }
        }
        Cmd::Unpeer { node_id } => {
            let sk = cairn_node::keystore::load(&cli.key, None)?;
            let db = cairn_node::db::connect(&cli.conn).await?;
            let id = cairn_node::identity::load_local(&db).await?;
            let kid = hex::encode(sk.verifying_key().to_bytes());
            cairn_node::identity::author_unpeer(
                &db, &sk, &kid, &id.node_id_hex, &node_id,
            ).await?;
            println!("unpeered {node_id}");
        }
        Cmd::Status => {
            let db = cairn_node::db::connect(&cli.conn).await?;
            let st = cairn_node::identity::status(&db, &cli.key).await?;
            println!("node_id       {}", st.node_id_hex);
            println!("peers_active  {}", st.peers_active);
            println!("peers_revoked {}", st.peers_revoked);
            println!("keystore_ok   {}", st.keystore_ok);
            if !st.keystore_ok {
                println!("              (cannot author: keystore unreadable)");
            }
            println!("key_at_rest   {}", st.key_at_rest);
            println!("runtime_role  {}", st.runtime_role);
            if st.db_floor_enforced {
                println!("db_floor      ENFORCED (connected role cannot raw-INSERT node_event)");
            } else {
                println!(
                    "db_floor      BYPASSABLE — '{}' can raw-INSERT node_event; \
                     run runtime as the cairn_node role to enforce the gate",
                    st.runtime_role
                );
            }
            println!("dr_escrow     {}", st.dr_escrow);
        }
        Cmd::Serve { listen } => {
            use cairn_node::sync;
            let sk = cairn_node::keystore::load(&cli.key, None)?;
            let db = cairn_node::db::connect(&cli.conn).await?;
            let trust = sync::trust_store_from_db(&db).await?;
            let (addr, serve_cfg) = sync::bind_serve(listen, &cli.conn, &sk, trust).await?;
            eprintln!("serving node_event sync on {addr}");
            sync::serve(serve_cfg).await?;
        }
        Cmd::Run { listen, peer, interval_secs } => {
            use cairn_node::sync;
            let sk = cairn_node::keystore::load(&cli.key, None)?;
            sync::run(listen, peer, &cli.conn, &sk, interval_secs).await?;
        }
    }
    Ok(())
}
