//! Resolving this node's custody (unwrap) key at daemon startup — issue #503.
//!
//! WHY THIS EXISTS. Every clinical body is born sealed (ADR-0052): its payload is
//! encrypted under a per-event DEK, and that DEK is wrapped to the node's X25519 *unwrap*
//! public key. Whoever holds the matching secret can open the record; whoever does not,
//! cannot. Until ADR-0066 that secret was HKDF-derived from the node's Ed25519 signing
//! seed, so any process holding the signing key could recompute it — which is exactly why
//! a disaster-recovery-restored node (fresh signing seed, by design) could never open its
//! own inherited records (#495).
//!
//! ADR-0066 made the unwrap key INDEPENDENT: `cairn-node` mints it, seals it into a
//! `<key>.unwrap` file, and registers its public half in `node_unwrap_key`. This module is
//! how `cairn-sync` gets at it — and, when it cannot, how it decides whether starting
//! anyway is safe.
//!
//! THE SHAPE OF THE DECISION. Three inputs meet here:
//!
//! 1. what the on-disk file yielded ([`FileOutcome`]),
//! 2. the key this daemon WOULD derive the old way, and
//! 3. the key the database has REGISTERED — the only authority on which key this node's
//!    existing DEKs are actually wrapped to.
//!
//! Input 3 is what makes a fallback admissible at all: a derived key that *equals the
//! registered key* is provably the right key, whatever its provenance. A derived key that
//! differs is provably the wrong one. So the daemon never has to guess.
//!
//! [`resolve`] is PURE — no file system, no database — so the whole table is proved by
//! unit tests. The IO that feeds it lives in [`load_file_outcome`].

use zeroize::Zeroizing;

/// What reading the `<key>.unwrap` file yielded. Three outcomes, not two: "there is no
/// file" and "there is a file I cannot use" lead to OPPOSITE decisions below, and
/// collapsing them is the defect #502 item 3 recorded.
pub enum FileOutcome {
    /// The file was read and unsealed; this is the secret it holds.
    Loaded(Zeroizing<[u8; 32]>),
    /// No file at the path.
    Absent,
    /// A file is present but could not be read, unsealed, or parsed. The `String` is the
    /// operator-facing cause, already legible.
    Unusable(String),
}

/// What the daemon should do about its custody key.
pub enum Resolution {
    /// Start, using `secret`. A `warning` is present only on the pre-ADR-0066 fallback,
    /// and must be printed on EVERY startup — see [`resolve`].
    Use {
        secret: Zeroizing<[u8; 32]>,
        warning: Option<String>,
    },
    /// Refuse to start, with this operator-facing message.
    Refuse(String),
}

/// Decide which unwrap secret this daemon should run with — or that it should not run.
///
/// `registered` is the public half stored in `node_unwrap_key`, or `None` when no row
/// exists (nothing has claimed custody on this node yet). `path_display` is the file path
/// to name in messages.
///
/// The table, and why each row is what it is:
///
/// | file | registered | outcome |
/// |---|---|---|
/// | `Loaded` | matches | use it — the normal post-ADR-0066 case |
/// | `Loaded` | diverges | **refuse** — wrong key file, or a node rebuilt under another |
/// | `Loaded` | none | use it — the provisioned file is the authority |
/// | `Absent` | matches derived | use derived, **warn loudly** — a pre-ADR-0066 node |
/// | `Absent` | diverges | **refuse** — a restored node; the #495 shape |
/// | `Absent` | none | use derived, silently — nothing is claimed; today's behaviour |
/// | `Unusable` | anything | **refuse** — never mask a corrupt custody file |
///
/// The `Unusable` row is the one worth pausing on. On an adopted node the derived key
/// would MATCH the registration, so falling back would work perfectly and hide the fact
/// that the `.unwrap` file has rotted — and that file is the only vehicle carrying this
/// node's custody off the machine (the sealed local-state export reads it). The node would
/// sync happily for months and discover the loss at restore, which is precisely the
/// "every surface honest, the composite a precise untruth" failure ADR-0066 exists to
/// correct. So: absent may fall back; unusable never may.
pub fn resolve(
    file: FileOutcome,
    derived: Zeroizing<[u8; 32]>,
    registered: Option<&[u8; 32]>,
    path_display: &str,
) -> Resolution {
    match file {
        FileOutcome::Unusable(cause) => Resolution::Refuse(format!(
            "the unwrap-key file {path_display} is present but unusable: {cause}. Refusing to \
             start. This daemon will NOT fall back to deriving a key, even if the derived one \
             would match what the database registered — that would mask the loss of the only \
             file carrying this node's custody off the machine (ADR-0066, issue #503). If the \
             file is sealed, set CAIRN_KEY_PASSPHRASE; if it is damaged, restore it from the \
             node's sealed local-state export before starting."
        )),
        FileOutcome::Loaded(secret) => match registered {
            Some(reg) if cairn_event::seal::unwrap_public(&secret) != *reg => {
                Resolution::Refuse(format!(
                    "the unwrap key in {path_display} is not the one this database registered \
                     (file {}, database {}). This daemon cannot open this node's custody. Point \
                     --key/--unwrap-key at the files this node was provisioned with (ADR-0066, \
                     issue #503).",
                    hex::encode(&cairn_event::seal::unwrap_public(&secret)[..8]),
                    hex::encode(&reg[..8]),
                ))
            }
            _ => Resolution::Use {
                secret,
                warning: None,
            },
        },
        FileOutcome::Absent => match registered {
            // Nothing claimed: today's behaviour, unchanged, and nothing to warn about.
            None => Resolution::Use {
                secret: derived,
                warning: None,
            },
            Some(reg) if cairn_event::seal::unwrap_public(&derived) == *reg => Resolution::Use {
                secret: derived,
                warning: Some(format!(
                    "WARNING: no unwrap-key file at {path_display}; falling back to the \
                     pre-ADR-0066 derived key. It matches what this database registered, so it \
                     IS the key this node's sealed records are wrapped to and custody works — \
                     but this node has no independent custody key on disk, so its sealed \
                     local-state export cannot carry one, and a disaster-recovery restore would \
                     lose custody of every sealed record. Run `cairn-node \
                     establish-unwrap-key` on this node to adopt the derived secret into a real \
                     key file. Do NOT run it on a node that has just been restored and whose \
                     export could not be read — see issue #503."
                )),
            },
            Some(reg) => Resolution::Refuse(format!(
                "no unwrap-key file at {path_display}, and the key this daemon would derive \
                 from its signing seed ({}) is not the one this database registered ({}). That \
                 is the signature of a node restored under a fresh signing seed: the derived \
                 key cannot open its inherited records, and using it would silently wrap new \
                 DEKs to a key nothing can read. Recover the node's `.unwrap` file from its \
                 sealed local-state export (ADR-0066, issue #503).",
                hex::encode(&cairn_event::seal::unwrap_public(&derived)[..8]),
                hex::encode(&reg[..8]),
            )),
        },
    }
}

/// Read the node's `<key>.unwrap` file and classify the result for [`resolve`].
///
/// THE CLASSIFICATION IS THE POINT. `Absent` may fall back to a derived key; `Unusable`
/// may not. Getting that boundary wrong in the permissive direction re-creates the exact
/// defect this module exists to prevent, so the recogniser is the error's TYPE and, for
/// IO, its [`std::io::ErrorKind`] — never its message text (a repo-wide convention: a
/// formatted message flattens the cause permanently and can never be taught to a
/// classifier).
///
/// Note which way each variant falls:
///
/// - `Io(NotFound)` — genuinely no file. **Absent.**
/// - `Io(_)` — a permissions error, an IO error, a directory where a file should be. The
///   file may well exist and be perfectly good; we simply could not read it. **Unusable.**
/// - `Sealed` — the file is there and valid, we just hold no secret for it. **Unusable**,
///   with the remedy named: this is the shape an operator hits by forgetting
///   `CAIRN_KEY_PASSPHRASE`, and silently deriving a key instead would be the worst
///   possible response to it.
/// - `Key(_)` — corrupt bytes, a wrong passphrase, a bundle that is neither sealed nor 32
///   bytes. **Unusable.**
pub fn load_file_outcome(path: &std::path::Path, passphrase: Option<&str>) -> FileOutcome {
    use cairn_keystore::keystore::KeystoreError;
    match cairn_keystore::keystore::load_unwrap_secret(path, passphrase) {
        Ok(secret) => FileOutcome::Loaded(secret),
        Err(KeystoreError::Io(e)) if e.kind() == std::io::ErrorKind::NotFound => {
            FileOutcome::Absent
        }
        Err(KeystoreError::Io(e)) => FileOutcome::Unusable(format!(
            "cannot read it ({e}) — the file may exist and be intact; this daemon could not \
             open it"
        )),
        Err(KeystoreError::Sealed) => FileOutcome::Unusable(
            "it is sealed and no passphrase was supplied — set CAIRN_KEY_PASSPHRASE to the \
             node's operational passphrase (or its recovery code)"
                .into(),
        ),
        Err(KeystoreError::Key(m)) => FileOutcome::Unusable(m),
    }
}

/// This node's custody identity for the life of one process: the Ed25519 signing key that
/// proves who it is, and the X25519 unwrap secret that opens what it holds.
///
/// WHY THEY TRAVEL TOGETHER. Presenting custody on the wire needs both at once — the
/// unwrap CERT is the node's unwrap public key SIGNED by its signing key, so a peer can
/// trust that re-wrapping a DEK for that public key gives it to this node and nobody else.
/// Passing them as two separate `Option`s let them disagree: before ADR-0066 the secret
/// was a pure function of the key so they could not, but that is exactly the coupling
/// ADR-0066 removed. One struct makes the mismatch unrepresentable.
pub struct NodeCustody {
    pub signing_key: cairn_event::SigningKey,
    pub unwrap_secret: Zeroizing<[u8; 32]>,
}

/// Where this daemon's unwrap-key file lives: the `<key>.unwrap` sibling of the signing
/// key by default — the same rule `cairn-node` uses to PLACE the file it provisions, so
/// the two agree without configuration — or the explicit `--unwrap-key` path when the
/// daemon's key file is not co-located with the node's.
///
/// Pure (no IO) so the rule is testable on its own.
pub fn unwrap_file_path(key_path: &str, override_path: Option<&str>) -> std::path::PathBuf {
    match override_path {
        Some(p) => std::path::PathBuf::from(p),
        None => cairn_keystore::keystore::unwrap_key_path_for(std::path::Path::new(key_path)),
    }
}

/// Resolve this daemon's custody key at startup, or refuse to start.
///
/// This is the one DB-touching step: it reads the registered public key, hands everything
/// to the pure [`resolve`], prints any warning, and returns the custody the process runs
/// with. Called from `cmd_pull`, `cmd_run` and the `serve` CLI arm — every command that
/// puts custody on the wire — BEFORE any network IO, so a misconfigured node fails at
/// startup rather than degrading into a peer that looks like it has no custody to offer.
pub fn resolve_at_startup(
    client: &mut postgres::Client,
    key_path: &str,
    unwrap_path_override: Option<&str>,
    signing_key: cairn_event::SigningKey,
) -> Result<NodeCustody, Box<dyn std::error::Error>> {
    let path = unwrap_file_path(key_path, unwrap_path_override);
    let display = path.display().to_string();

    // `query_opt`, not `query_one`: an absent row is a legitimate "nothing claimed".
    let row = client.query_opt("SELECT unwrap_pub FROM node_unwrap_key", &[])?;
    let registered: Option<[u8; 32]> = match row.map(|r| r.get::<_, Vec<u8>>(0)) {
        None => None,
        Some(bytes) => {
            // A ROW THAT IS NOT 32 BYTES IS A REFUSAL, NEVER "nothing registered".
            // db/037 CHECKs this column, so the branch is unreachable today — but a
            // daemon that does not own this schema must not stake a safety gate on a
            // constraint in it, and folding a malformed row into `None` would be
            // fail-OPEN inside a fail-fast gate.
            Some(bytes.as_slice().try_into().map_err(|_| {
                format!(
                    "node_unwrap_key.unwrap_pub is {} bytes, not 32 — this database's custody \
                     plane is malformed and this daemon cannot tell whether its own unwrap key \
                     agrees with it. Refusing to start rather than proceeding as though no key \
                     were registered.",
                    bytes.len()
                )
            })?)
        }
    };

    // The passphrase for a sealed key file, the same environment variable cairn-node
    // reads. Absent, a sealed file classifies Unusable (with the remedy named) — it never
    // degrades into "absent" and a derived key.
    let passphrase = std::env::var("CAIRN_KEY_PASSPHRASE").ok();
    let file = load_file_outcome(&path, passphrase.as_deref());
    let derived = cairn_event::seal::derive_unwrap_secret(&signing_key.to_bytes());

    match resolve(file, derived, registered.as_ref(), &display) {
        Resolution::Refuse(message) => Err(message.into()),
        Resolution::Use { secret, warning } => {
            if let Some(w) = warning {
                // Every startup, deliberately. A once-only notice is how a deleted key
                // file stays invisible until a restore needs it.
                eprintln!("cairn-sync: {w}");
            }
            Ok(NodeCustody {
                signing_key,
                unwrap_secret: secret,
            })
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use cairn_event::seal::unwrap_public;
    use zeroize::Zeroizing;

    /// House rule 6: key material is DERIVED at runtime, never a literal, so CodeQL's
    /// hard-coded-cryptographic-value query stays live for production code.
    fn secret_fixture(tag: u8) -> Zeroizing<[u8; 32]> {
        Zeroizing::new(std::array::from_fn(|i| tag ^ (i as u8).wrapping_mul(7)))
    }

    #[test]
    fn a_loaded_file_matching_the_registration_is_used() {
        let provisioned = secret_fixture(0x11);
        let registered = unwrap_public(&provisioned);
        let r = resolve(
            FileOutcome::Loaded(provisioned.clone()),
            secret_fixture(0x22),
            Some(&registered),
            "node.key.unwrap",
        );
        match r {
            Resolution::Use { secret, warning } => {
                assert_eq!(*secret, *provisioned, "the FILE's secret is the one used");
                assert!(
                    warning.is_none(),
                    "a loaded key is the normal case: no warning"
                );
            }
            Resolution::Refuse(m) => panic!("the happy path must not refuse: {m}"),
        }
    }

    #[test]
    fn a_loaded_file_diverging_from_the_registration_refuses() {
        let registered = unwrap_public(&secret_fixture(0x33));
        let r = resolve(
            FileOutcome::Loaded(secret_fixture(0x44)),
            secret_fixture(0x55),
            Some(&registered),
            "node.key.unwrap",
        );
        let Resolution::Refuse(m) = r else {
            panic!("a divergent key must refuse");
        };
        assert!(
            m.contains("node.key.unwrap"),
            "the refusal names the file: {m}"
        );
    }

    #[test]
    fn a_loaded_file_with_nothing_registered_is_used() {
        // No row in node_unwrap_key: nothing has been claimed, and the provisioned
        // file is the authority on what this node's custody key is.
        let provisioned = secret_fixture(0x66);
        let r = resolve(
            FileOutcome::Loaded(provisioned.clone()),
            secret_fixture(0x77),
            None,
            "node.key.unwrap",
        );
        let Resolution::Use { secret, warning } = r else {
            panic!("nothing registered is not a refusal");
        };
        assert_eq!(*secret, *provisioned);
        assert!(warning.is_none());
    }

    #[test]
    fn an_absent_file_whose_derived_key_matches_falls_back_loudly() {
        // The pre-ADR-0066 node. Its registered key IS the derived one, so the derived
        // secret is provably the key its event_dek rows are wrapped to — using it is
        // correct, not a reintroduction of the #495 coupling.
        let derived = secret_fixture(0x88);
        let registered = unwrap_public(&derived);
        let r = resolve(
            FileOutcome::Absent,
            derived.clone(),
            Some(&registered),
            "node.key.unwrap",
        );
        let Resolution::Use { secret, warning } = r else {
            panic!("a provable pre-ADR-0066 node must start");
        };
        assert_eq!(*secret, *derived);
        let w = warning.expect("the fallback is LOUD — a silent one hides a deleted key file");
        assert!(
            w.contains("node.key.unwrap"),
            "the warning names the missing file: {w}"
        );
        assert!(
            w.contains("establish-unwrap-key"),
            "the warning names the remedy: {w}"
        );
    }

    #[test]
    fn an_absent_file_whose_derived_key_diverges_refuses() {
        // The RESTORED node: fresh signing seed, so the derived key cannot be the one
        // its inherited event_dek rows are wrapped to. This is the #495 shape, caught.
        let registered = unwrap_public(&secret_fixture(0x99));
        let r = resolve(
            FileOutcome::Absent,
            secret_fixture(0xAA),
            Some(&registered),
            "node.key.unwrap",
        );
        assert!(
            matches!(r, Resolution::Refuse(_)),
            "a restored node must never proceed on a derived key"
        );
    }

    #[test]
    fn an_absent_file_with_nothing_registered_uses_the_derived_key_silently() {
        // Today's behaviour, preserved exactly: nothing has claimed custody, so there
        // is nothing for this daemon to be wrong about, and no warning to give.
        let derived = secret_fixture(0xBB);
        let r = resolve(
            FileOutcome::Absent,
            derived.clone(),
            None,
            "node.key.unwrap",
        );
        let Resolution::Use { secret, warning } = r else {
            panic!("an unprovisioned node must still start");
        };
        assert_eq!(*secret, *derived);
        assert!(warning.is_none(), "no claim, no warning");
    }

    #[test]
    fn an_unusable_file_refuses_and_never_falls_back() {
        // #502 item 3, sharpened: on an adopted node a successful derive would MASK a
        // corrupt .unwrap file, and that file is the only vehicle carrying custody off
        // the machine. Present-but-unusable is not "absent".
        let derived = secret_fixture(0xCC);
        let registered = unwrap_public(&derived); // the derive WOULD have matched
        let r = resolve(
            FileOutcome::Unusable("wrong passphrase or corrupt file".into()),
            derived,
            Some(&registered),
            "node.key.unwrap",
        );
        let Resolution::Refuse(m) = r else {
            panic!("a corrupt key file must refuse even when the derive would match");
        };
        assert!(
            m.contains("wrong passphrase"),
            "the refusal carries the cause: {m}"
        );
    }

    #[test]
    fn an_unusable_file_refuses_with_nothing_registered_too() {
        let r = resolve(
            FileOutcome::Unusable("not a sealed bundle".into()),
            secret_fixture(0xDD),
            None,
            "node.key.unwrap",
        );
        assert!(
            matches!(r, Resolution::Refuse(_)),
            "a garbage key file is a defect whether or not custody is claimed"
        );
    }

    #[test]
    fn a_missing_file_classifies_as_absent() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("node.key.unwrap");
        assert!(
            matches!(load_file_outcome(&path, None), FileOutcome::Absent),
            "a file that is not there is Absent, never Unusable"
        );
    }

    #[test]
    fn a_plaintext_secret_file_loads() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("node.key.unwrap");
        let secret = cairn_event::seal::generate_unwrap_secret().unwrap();
        cairn_keystore::keystore::write_unwrap_plaintext(&path, &secret).unwrap();
        let FileOutcome::Loaded(loaded) = load_file_outcome(&path, None) else {
            panic!("a plaintext unwrap key must load");
        };
        assert_eq!(*loaded, *secret, "the bytes cross the disk intact");
    }

    #[test]
    fn a_sealed_file_loads_with_its_passphrase() {
        // The cross-crate link that nothing tests today: cairn-node SEALS the file and
        // cairn-sync must OPEN it. DR slice 1's lesson 4 — where no test carries the
        // value across the disk, the one link that matters is proven by nothing.
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("node.key.unwrap");
        let pass = "op-passphrase-for-this-test";
        let code = cairn_keystore::seal::generate_recovery_code();
        let public = cairn_keystore::keystore::generate_unwrap_sealed(&path, pass, &code).unwrap();
        let FileOutcome::Loaded(loaded) = load_file_outcome(&path, Some(pass)) else {
            panic!("a sealed unwrap key must load under its passphrase");
        };
        assert_eq!(
            cairn_event::seal::unwrap_public(&loaded),
            public,
            "the secret loaded is the one whose public half was registered"
        );
    }

    #[test]
    fn a_sealed_file_without_a_passphrase_is_unusable_not_absent() {
        // The case that MUST NOT fall back: the key is right there, we simply have no
        // secret for it. Treating this as "absent" would derive a key and carry on.
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("node.key.unwrap");
        let code = cairn_keystore::seal::generate_recovery_code();
        cairn_keystore::keystore::generate_unwrap_sealed(&path, "op-pass", &code).unwrap();
        let FileOutcome::Unusable(cause) = load_file_outcome(&path, None) else {
            panic!("a sealed file with no passphrase is Unusable, never Absent");
        };
        assert!(
            cause.contains("CAIRN_KEY_PASSPHRASE"),
            "the cause names the remedy: {cause}"
        );
    }

    #[test]
    fn a_corrupt_file_is_unusable() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("node.key.unwrap");
        std::fs::write(&path, b"not a sealed bundle and not 32 bytes").unwrap();
        assert!(
            matches!(load_file_outcome(&path, None), FileOutcome::Unusable(_)),
            "garbage is Unusable"
        );
    }

    #[test]
    fn a_wrong_passphrase_is_unusable() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("node.key.unwrap");
        let code = cairn_keystore::seal::generate_recovery_code();
        cairn_keystore::keystore::generate_unwrap_sealed(&path, "right-pass", &code).unwrap();
        assert!(
            matches!(
                load_file_outcome(&path, Some("wrong-pass")),
                FileOutcome::Unusable(_)
            ),
            "a wrong passphrase must never degrade into Absent"
        );
    }

    #[test]
    fn the_override_wins_over_the_sibling_default() {
        let dir = tempfile::tempdir().unwrap();
        let elsewhere = dir.path().join("custody.key");
        assert_eq!(
            unwrap_file_path("/nodes/a/node.key", Some(elsewhere.to_str().unwrap())),
            elsewhere,
            "--unwrap-key names the file outright"
        );
    }

    #[test]
    fn the_default_is_the_sibling_of_the_signing_key() {
        assert_eq!(
            unwrap_file_path("/nodes/a/node.key", None),
            std::path::Path::new("/nodes/a/node.key.unwrap"),
            "same rule cairn-node uses to place the file it provisions"
        );
    }
}
