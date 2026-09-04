//! ADR-0026 slice D — integration tests for the sealed local-state export.
//! DB-gated tests need CAIRN_TEST_PG (local PG with cairn_pgx installed); offline tests
//! always run.

use cairn_node::db;
use cairn_node::localstate::{
    apply_local_state, establish_lsk, from_cbor, localstate_path_for, lsk_sidecar_path_for,
    parse_container, read_local_state, seal_local_state, serialize_container, serialize_sidecar,
    to_cbor, unseal_local_state_rec, CustodyKeyDestination, LocalState,
};
use tempfile::tempdir;

fn cs() -> Option<String> {
    std::env::var("CAIRN_TEST_PG").ok()
}

/// The round-trip's READ half, over a node that genuinely holds nothing to export.
///
/// The emptiness asserted here is **correct, not a defect**. `reset_node_federation_tables`
/// runs first, so there is no `event_dek` custody to carry; and `None` is passed for the
/// unwrap secret, which is what a caller that could not load `<key>.unwrap` supplies. An
/// empty bundle is the right answer to both, and applying one must be a clean noop.
///
/// **What this test is NOT.** It is not evidence that the export is empty in general —
/// since #495/ADR-0066 a provisioned node's export carries its surviving `event_dek`
/// custody and its unwrap secret. That property is pinned where it can actually be
/// observed, against a database holding real custody, in
/// `dr_clinical_guarantee_gap.rs::the_export_carries_the_unwrap_secret_and_the_surviving_dek`.
/// If this test ever stops resetting the federation tables, it stops being true.
///
/// (Two earlier framings of this test were wrong in turn — first "no clinical surface yet,
/// so the tier's bundle is empty", falsified by ADR-0052's born-sealed bodies; then "the
/// emptiness is a defect", falsified by ADR-0066. The name was already changed once off
/// `..._is_empty_at_the_federation_tier` for the first of those, because a test name is a
/// claim. The stable claim is the one above: this node has nothing, so it exports nothing.)
#[tokio::test]
async fn read_local_state_returns_the_empty_bundle() {
    let Some(base) = cs() else {
        eprintln!("skipped: set CAIRN_TEST_PG");
        return;
    };
    let _guard = db::test_serial_guard(&base).await.unwrap();
    let conn = db::connect_and_load_schema(&base).await.unwrap();
    db::reset_node_federation_tables(&conn).await.ok();
    // The CLINICAL custody plane too, and this line is load-bearing since #495. Before it,
    // `read_local_state` ignored the database entirely, so "no custody" needed no setup.
    // Now the answer depends on `event_dek` — and `reset_node_federation_tables` truncates
    // only the FEDERATION tables (`node_event`, `local_node`, `sync_cursor`, `hlc_state`,
    // `node_event_quarantine`), never this one. These integration binaries share one
    // serialized database, so a sibling suite's leftover custody row would otherwise make
    // this test fail depending on binary order.
    conn.batch_execute("TRUNCATE event_dek, erasure_shred_log CASCADE")
        .await
        .expect("clearing the custody plane so this node genuinely holds nothing");

    // `None`: the caller could not load an unwrap secret. The export is still built — see
    // `read_local_state`'s doc for why that is a warn, not an abort.
    let ls = read_local_state(&conn, None)
        .await
        .expect("read must succeed");
    assert!(
        ls.is_empty(),
        "a node with no custody and no secret to carry exports an empty bundle"
    );
    // Applying an empty bundle is a clean noop. Since ADR-0066 decision 4 the applier
    // INSTALLS a carried unwrap key, so it needs a destination to install it at — but this
    // bundle carries none, so nothing is written and the tempdir path below is never touched.
    // Asserting that is the point: "no secret to install" must stay distinguishable from
    // "installed something", and the report is where a caller reads the difference.
    let dir = tempdir().unwrap();
    let report = apply_local_state(
        &conn,
        &ls,
        &CustodyKeyDestination::Sealed {
            path: &dir.path().join("node.key.unwrap"),
            op_pass: "op-pass-for-a-bundle-that-carries-nothing",
            recovery_code: "recovery-code-for-a-bundle-that-carries-nothing",
        },
    )
    .await
    .expect("applying an empty bundle is a noop");
    assert_eq!(
        report.unwrap_key_installed(),
        None,
        "an empty bundle installs no custody key — and must say so rather than claim one"
    );
    assert_eq!(report.episode_deks_carried(), 0);
    assert!(
        !dir.path().join("node.key.unwrap").exists(),
        "nothing may be written when there is nothing to install"
    );
}

#[test]
fn export_then_restore_roundtrips_an_empty_bundle_offline() {
    // Pure/offline slice of the round-trip (no DB): seal an empty bundle under an LSK,
    // write the CAIRNL1 sibling, then unseal it via the recovery code and apply-check it.
    let dir = tempdir().unwrap();
    let medium = dir.path().join("cairn.medium");
    let op = "op-pass";
    let code = "AB12C-D34EF";

    let wraps = establish_lsk(op, code).unwrap();
    let bundle = to_cbor(&LocalState::empty());
    let sealed = seal_local_state(&wraps, op, &bundle).unwrap();
    let export_path = localstate_path_for(&medium);
    std::fs::write(&export_path, serialize_container(&sealed)).unwrap();

    // Restore side: read the sibling, unseal with the OLD recovery code, decode, check empty.
    let bytes = std::fs::read(&export_path).unwrap();
    let parsed = parse_container(&bytes).unwrap();
    let plaintext = unseal_local_state_rec(&parsed, code).expect("recovery code must unseal");
    let restored = from_cbor(&plaintext).unwrap();
    assert!(restored.is_empty(), "an empty bundle restores empty");
}

#[test]
fn sidecar_written_atomically_is_readable() {
    // The `.lsk` escrow the CLI writes must parse back (guards the serialize/atomic-write pair).
    let dir = tempdir().unwrap();
    let key = dir.path().join("node.key");
    let wraps = establish_lsk("op", "REC-CODE").unwrap();
    cairn_node::fsio::atomic_write(
        &lsk_sidecar_path_for(&key),
        &serialize_sidecar(&wraps),
        Some(0o600),
    )
    .unwrap();
    let back = std::fs::read(lsk_sidecar_path_for(&key)).unwrap();
    assert!(cairn_node::localstate::parse_sidecar(&back).is_ok());
}

#[test]
fn corrupt_container_parses_as_error_not_panic() {
    // A bit-rotted export sibling must surface as Err so restore can WARN+skip
    // (honest degradation) rather than bailing an already-restored node.
    let garbage = b"CAIRNL1\nnot valid cbor at all";
    assert!(cairn_node::localstate::parse_container(garbage).is_err());
    assert!(cairn_node::localstate::parse_container(b"no magic here").is_err());
}

/// **The evidence behind `restore`'s reworded failure line (review finding C1).**
///
/// `parse_container` validates the `CAIRNL1` magic and the CBOR framing — and nothing
/// else. `payload_ct` is an opaque byte string inside that frame, so corruption *within*
/// the sealed body sails through the parse and surfaces only when the AEAD tag fails to
/// verify: `unseal_local_state_rec` returns `None`, which is **bit-for-bit the same answer
/// a wrong recovery code produces**.
///
/// That is why the restore arm may not tell an operator *"the export itself is intact;
/// the code is what failed"* after a failed unseal. It cannot know that, and saying so
/// sends someone hunting a code they already typed correctly and then spending a second
/// superseding identity for nothing — a precise untruth in the reassuring direction, at
/// the one moment the operator has no second attempt (principle 4).
///
/// This test uses the CORRECT code throughout, so a `None` here can only mean damage.
#[test]
fn ciphertext_damage_is_indistinguishable_from_a_wrong_recovery_code() {
    let op = "op-passphrase";
    let code = "REC-CODE-FIXTURE";
    let wraps = establish_lsk(op, code).unwrap();
    let bundle = LocalState::empty();
    let sealed = seal_local_state(&wraps, op, &to_cbor(&bundle)).unwrap();
    let container = serialize_container(&sealed);

    // Positive control: undamaged, the correct code recovers the bundle. Without this the
    // assertions below would pass just as well against a container that never sealed
    // anything.
    let intact = parse_container(&container).expect("an undamaged container must parse");
    assert!(
        unseal_local_state_rec(&intact, code).is_some(),
        "precondition: the correct recovery code must open an undamaged export"
    );

    // Damage the ciphertext BEFORE serialization, not by flipping a byte of the already-
    // serialized container.
    //
    // An earlier version of this test flipped the LAST byte of the serialized container,
    // reasoning that the ciphertext is the largest run so the tail must land inside it,
    // "past every CBOR key, and therefore invisible to the parser." That reasoning was
    // wrong: `SealedLocalState.payload_ct` (localstate.rs) is a plain `Vec<u8>` with no
    // `serde_bytes` attribute, so ciborium encodes it not as a CBOR byte string but as a
    // CBOR ARRAY OF INTEGERS — one structural element per byte. Ciphertext bytes are
    // therefore parser-VISIBLE, not invisible: whenever the damaged byte happens to be
    // < 24 (0x18), it round-trips as a single-byte CBOR integer, and `^= 0xFF` turns it
    // into major type 7 (a CBOR "simple" value), which `parse_container` rejects as
    // malformed framing instead of the frame surviving intact. That is roughly a 1-in-11
    // chance per run (P(byte < 24) = 24/256 ~= 9.4%) — measured as 7 failures in 40
    // consecutive local runs (~17%, consistent with the last byte of ciphertext not being
    // uniform), and it flaked CI on an unrelated PR.
    //
    // The honest fix: mutate `sealed.payload_ct` (still a plain in-memory `Vec<u8>` at this
    // point) BEFORE calling `serialize_container`, so the frame is intact by construction —
    // never by an assumption about how ciborium happens to encode an untyped byte vector.
    let mut damaged_ct = sealed.payload_ct().to_vec();
    let last = damaged_ct.len() - 1;
    damaged_ct[last] ^= 0xFF;
    // Rebuilt through `SealedLocalState::new` rather than by poking a field: since #511 the
    // fields are `pub(crate)`, so assembling one is a deliberate act that names all three
    // parts (rides-along 3). The damage is identical; only the way it is expressed moved.
    let sealed_damaged = cairn_node::localstate::SealedLocalState::new(
        sealed.wraps().clone(),
        *sealed.payload_nonce(),
        damaged_ct,
    );
    let damaged = serialize_container(&sealed_damaged);

    let parsed = parse_container(&damaged)
        .expect("THE POINT: ciphertext damage still parses — the frame is intact");
    assert!(
        unseal_local_state_rec(&parsed, code).is_none(),
        "damaged ciphertext must fail to unseal even under the CORRECT code"
    );
}

// ---------------------------------------------------------------------------------------
// #511 rides-along: the producer set, and structural redaction
// ---------------------------------------------------------------------------------------

/// **`from_custody` fills the custody slot, and the narrow mutators cannot.**
///
/// `LocalState`'s own doc has always said that a third producer skipping the
/// `erasure_shred_log` filter "is how an erased body's key would travel" — and until #511
/// nothing prevented one, because every field was `pub`. There is deliberately no
/// `set_episode_deks`: that is the slot the filter guards, and the only way to fill it is
/// [`LocalState::from_custody`], which `read_local_state` (the filtering producer) calls.
///
/// ⚠️ **This test does NOT prove the "exactly one filler" part, and its name used to claim it
/// did.** The body below is a positive round-trip: it would pass unchanged if a third producer
/// appeared tomorrow. The exclusivity claim is enforced somewhere else entirely — by
/// `dr_clinical_guarantee_gap.rs`'s `local_state_producers_are_the_two_named_constructors`,
/// which sweeps every crate's `src/` and counts constructions. Named here so a future
/// maintainer weakening or deleting that guard does not assume this test still covers it
/// (round-1 review of #511; a test name is a claim).
///
/// The narrow mutators that DO exist cover slots the filter has nothing to say about.
#[test]
fn from_custody_is_the_only_way_to_fill_the_custody_slot() {
    let secret = cairn_event::keys::Secret32::from_bytes(std::array::from_fn(|i| {
        (i as u8).wrapping_mul(7).wrapping_add(1)
    }));
    let ls = LocalState::from_custody(vec![b"a wrapped row".to_vec()], Some(secret.clone()));
    assert_eq!(ls.episode_deks().len(), 1);
    assert_eq!(ls.unwrap_secret(), Some(&secret));
    assert!(
        !ls.is_empty(),
        "a bundle carrying custody is not the zero value"
    );

    // And the zero value stays the zero value.
    assert!(LocalState::empty().is_empty());
    assert_eq!(LocalState::empty().episode_deks().len(), 0);
    assert_eq!(LocalState::empty().unwrap_secret(), None);
}

/// **Redaction is structural now, not per-field.**
///
/// The hand-written `Debug` that existed only to redact `unwrap_secret` is gone: the field is
/// a `Secret32`, whose own `Debug` prints `Secret32(<redacted>)`, so `#[derive(Debug)]` is
/// safe again. The difference that matters is not the output — it is that the NEXT
/// secret-bearing slot added to this struct inherits the redaction instead of having to
/// re-earn it, which is the failure mode #511 named for the `Drop` impl beside it.
#[test]
fn debug_redacts_the_secret_without_a_hand_written_impl() {
    let raw: [u8; 32] = std::array::from_fn(|i| (i as u8).wrapping_mul(11).wrapping_add(3));
    let mut ls = LocalState::empty();
    ls.set_unwrap_secret(Some(cairn_event::keys::Secret32::from_bytes(raw)));

    let shown = format!("{ls:?}");
    assert!(
        shown.contains("<redacted>"),
        "the secret must be redacted: {shown}"
    );
    assert!(
        !shown.contains(&hex::encode(raw)[..8]),
        "the bundle's Debug leaked the node's custody key: {shown}"
    );
    assert!(
        shown.contains("Some("),
        "presence must still be visible — a reader has to tell 'absent' from 'hidden'"
    );
    assert!(
        !format!("{:?}", LocalState::empty()).contains("<redacted>"),
        "…and an absent secret must not look like a hidden one"
    );
}

/// **`take_unwrap_secret` is a move, not a copy.**
///
/// It exists because `restore`-shaped tests need to lift the secret out of a bundle; the
/// point pinned here is that the bundle is genuinely left without one afterwards, so a
/// caller cannot accidentally leave a second live copy of the node's custody key behind.
#[test]
fn taking_the_secret_leaves_the_bundle_without_one() {
    let secret = cairn_event::keys::Secret32::from_bytes(std::array::from_fn(|i| {
        (i as u8).wrapping_mul(5).wrapping_add(2)
    }));
    let mut ls = LocalState::empty();
    ls.set_unwrap_secret(Some(secret.clone()));
    assert_eq!(ls.take_unwrap_secret(), Some(secret));
    assert_eq!(ls.unwrap_secret(), None);
    assert!(ls.is_empty(), "and the bundle is back to its zero value");
}
