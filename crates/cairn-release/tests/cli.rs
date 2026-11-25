//! The `cairn-release` binary end to end, in a temp dir with a runtime-generated passphrase.

use cairn_release::{check_artifact, decode, genesis, verify_bundle, BundleFile, SignedRoot};
use std::path::Path;
use std::process::{Command, Output};

const PASS_VAR: &str = "CAIRN_RELEASE_TEST_PASS";

fn run(args: &[&str]) -> Output {
    // The passphrase is derived at runtime, never a literal (house rule 6).
    let pass: String = (0..12)
        .map(|i| (b'a' + (i * 7 % 26) as u8) as char)
        .collect();
    Command::new(env!("CARGO_BIN_EXE_cairn-release"))
        .args(args)
        .arg("--passphrase-env")
        .arg(PASS_VAR)
        .env(PASS_VAR, pass)
        .output()
        .unwrap()
}

fn ok(args: &[&str]) -> String {
    let o = run(args);
    assert!(o.status.success(), "{}", String::from_utf8_lossy(&o.stderr));
    String::from_utf8(o.stdout).unwrap().trim().to_string()
}

fn s(p: &Path) -> &str {
    p.to_str().unwrap()
}

#[test]
fn keygen_then_genesis_then_bundle_round_trips() {
    let d = tempfile::tempdir().unwrap();
    let (rk, ak) = (d.path().join("root.key"), d.path().join("rel.key"));
    let rpub = ok(&["keygen", "--out", s(&rk)]);
    let apub = ok(&["keygen", "--out", s(&ak)]);
    let g = d.path().join("genesis.cbor");
    ok(&[
        "genesis",
        "--channel",
        "official",
        "--root-signer",
        &rpub,
        "--root-threshold",
        "1",
        "--release-signer",
        &apub,
        "--release-threshold",
        "1",
        "--key",
        s(&rk),
        "--out",
        s(&g),
    ]);
    let art = d.path().join("window.bin");
    std::fs::write(&art, b"a window program").unwrap();
    let side = d.path().join("window.json");
    std::fs::write(
        &side,
        r#"{"generation_line":"gen-1","release_version":5,"platform":"debian-12"}"#,
    )
    .unwrap();
    let out = d.path().join("release.cbor");
    ok(&[
        "bundle",
        "--history",
        s(&g),
        "--key",
        s(&ak),
        "--artifact",
        s(&art),
        "--sidecar",
        s(&side),
        "--release-version",
        "5",
        "--out",
        s(&out),
    ]);

    // Verify it the way a terminal would.
    let root: SignedRoot = decode(&std::fs::read(&g).unwrap()).unwrap();
    let h = vec![genesis(&root).unwrap()];
    let b: BundleFile = decode(&std::fs::read(&out).unwrap()).unwrap();
    let (_, m) = verify_bundle(&h, &b.meta, 0).unwrap();
    assert_eq!(m.release_version, 5);
    assert_eq!(m.artifacts[0].generation_line.as_deref(), Some("gen-1"));
    assert_eq!(m.artifacts[0].platform.as_deref(), Some("debian-12"));
    check_artifact(&m, 0, b"a window program").unwrap();
}

#[test]
fn the_cli_refuses_a_sidecar_whose_release_differs() {
    let d = tempfile::tempdir().unwrap();
    let k = d.path().join("k.key");
    let kp = ok(&["keygen", "--out", s(&k)]);
    let g = d.path().join("g.cbor");
    ok(&[
        "genesis",
        "--channel",
        "c",
        "--root-signer",
        &kp,
        "--root-threshold",
        "1",
        "--release-signer",
        &kp,
        "--release-threshold",
        "1",
        "--key",
        s(&k),
        "--out",
        s(&g),
    ]);
    let art = d.path().join("w.bin");
    std::fs::write(&art, b"x").unwrap();
    let side = d.path().join("window.json");
    std::fs::write(
        &side,
        r#"{"generation_line":"g","release_version":4,"platform":"p"}"#,
    )
    .unwrap();
    let out = d.path().join("r.cbor");
    let o = run(&[
        "bundle",
        "--history",
        s(&g),
        "--key",
        s(&k),
        "--artifact",
        s(&art),
        "--sidecar",
        s(&side),
        "--release-version",
        "5",
        "--out",
        s(&out),
    ]);
    assert!(!o.status.success());
    assert!(!out.exists(), "nothing is written for a refused bundle");
}
