//! Golden CBOR bytes for the `CAIRNL1` local-state bundle — issue #511.
//!
//! WHY THIS EXISTS. `LocalState::unwrap_secret` changes Rust type in this slice
//! (`Option<Vec<u8>>` → `Option<Secret32>`), and it is a **serialized** field of the export a
//! restored clinic reads its custody out of. A round-trip test proves nothing about that: it
//! encodes and decodes through the same pair, so a mirrored change on both sides stays green.
//! That is not hypothetical — DR slice 2a found **19 of 19** single-line mutations surviving a
//! green suite for precisely this reason, and only golden bytes killed them. So the pin below
//! is captured from the build that exists BEFORE the type moves.
//!
//! WHEN THIS FAILS. The on-disk export format has moved, and every `.localstate` file any
//! operator is holding off-site was written in the old shape. **Do not re-freeze the constant
//! to make it green** — that is the exact failure this test exists to prevent. Either the
//! change is wrong, or it is a deliberate format break that needs a version bump, a migration
//! story, and an ADR.
//!
//! ON HOUSE RULE 6, stated because this file does the one thing `cairn-medium/src/wire_pins.rs`
//! says a golden pin should avoid — its frozen bytes contain a 32-byte value sitting in a
//! secret-shaped slot. There is no way around it: `unwrap_secret`'s ENCODING is the thing under
//! test, so its bytes must appear. What makes it safe is the two halves of the rule, both
//! satisfied: the value is not cryptographic (it is derived at runtime by [`secret_fixture`],
//! opens nothing, and reaches no KDF, cipher, or signer — only `hex::decode` and a CBOR
//! parser), and the binding it flows into is `POPULATED_BUNDLE_CBOR_HEX`, not a
//! `salt`/`nonce`/`iv` sink name. Every other slot uses a SHORT placeholder, exactly as
//! `wire_pins.rs` does, so the constant stays reviewable and the one field that matters is the
//! one pinned in full.

use cairn_event::keys::Secret32;
use cairn_node::localstate::{episode_dek_to_cbor, from_cbor, to_cbor, EpisodeDek, LocalState};

/// A 32-byte secret, derived at runtime — house rule 6(a): never a byte-array literal in a
/// crypto context, even in a fixture. `lineage` is a discriminator, deliberately NOT called
/// `salt`: CodeQL picks its sink by the NAME a value flows into, and a discriminator wearing a
/// KDF's name mints a critical alert per call site (#527).
fn secret_fixture(lineage: u8) -> Secret32 {
    Secret32::from_bytes(std::array::from_fn(|i| {
        (i as u8).wrapping_mul(7).wrapping_add(lineage)
    }))
}

/// A SHORT placeholder for the wrapped-custody slot, following `cairn-medium`'s wire-pin
/// convention: pin the field under test in full, keep everything else small enough to read.
///
/// The real wrapped DEK is `WRAPPED_DEK_LEN` = 104 bytes, whose CBOR length prefix is the
/// two-byte `0x98 0x68` form. Nothing is lost by shortening it here, because `unwrap_secret`
/// — the field whose Rust type this slice changes — is itself 32 bytes and so exercises that
/// same `0x98` prefix path. A 104-byte row would add ~500 hex characters of noise to a
/// constant a reviewer has to be able to look at.
fn wrapped_dek_placeholder() -> Vec<u8> {
    (0u8..8).map(|i| i.wrapping_mul(3)).collect()
}

/// The bundle the golden bytes below encode.
///
/// **Every content slot is populated**, and that is load-bearing: a pin taken over
/// `LocalState::empty()` would pin almost nothing (five empty containers and a version byte),
/// and `unwrap_secret`'s encoding — the one field whose Rust type changes — would not appear in
/// it at all.
fn populated_bundle() -> LocalState {
    let mut ls = LocalState::from_custody(
        vec![episode_dek_to_cbor(&EpisodeDek {
            event_id: "00000000-0000-0000-0000-000000000001".to_string(),
            dek_wrapped: wrapped_dek_placeholder(),
        })],
        Some(secret_fixture(5)),
    );
    ls.set_config(Some(b"config-blob".to_vec()));
    ls.set_drafts(vec![b"a draft".to_vec()]);
    ls
}

/// The exact CBOR a pre-#511 build produces for [`populated_bundle`].
///
/// Frozen 2026-09-04, from the tree at the commit that introduced this file. See the module
/// header before changing a single character of it.
const POPULATED_BUNDLE_CBOR_HEX: &str = concat!(
    // map(6) — the six LocalState fields, in declaration order
    "a6",
    // "version" -> 1
    "6776657273696f6e01",
    // "node_default_deks" -> array(0)
    "716e6f64655f64656661756c745f64656b7380",
    // "episode_deks" -> array(1) of array(69)
    "6c657069736f64655f64656b73819845",
    //   the EpisodeDek CBOR, one uint per byte (the SHORT placeholder row)
    "18a21868186518761865186e1874185f1869186418781824183018301830183018301830",
    "18301830182d1830183018301830182d1830183018301830182d1830183018301830182d",
    "183018301830183018301830183018301830183018301831186b18641865186b185f1877",
    "1872186118701870186518641888000306090c0f1215",
    // "config" -> array(11) = b"config-blob"
    "66636f6e6669678b1863186f186e186618691867182d1862186c186f1862",
    // "drafts" -> array(1) of array(7) = b"a draft"
    "6664726166747381871861182018641872186118661874",
    // "unwrap_secret" -> 0x98 0x20 = array(32)  <-- THE FIELD UNDER TEST
    "6d756e777261705f7365637265749820",
    //   its 32 bytes, one uint each — the encoding Secret32 must reproduce
    "050c13181a18211828182f1836183d1844184b1852185918601867186e1875187c188318",
    "8a18911898189f18a618ad18b418bb18c218c918d018d718de",
);

/// The EMPTY bundle, whose `unwrap_secret` is `None`.
///
/// **Why a second constant, when the populated one already covers the interesting field.**
/// `Option`'s encoding is decided by serde before `Secret32`'s `Serialize` is ever reached, so
/// `None` cannot vary with the inner type and the risk here is genuinely small. But this file's
/// whole thesis is that a round-trip proves nothing, and until round-1 review of #511 the `None`
/// case was covered by round-trips alone (`localstate.rs`'s `empty_bundle_cbor_roundtrips`). What
/// this pins is cheap and real: the field NAMES, their ORDER, the version byte, and that an
/// absent secret is CBOR `null` (`f6`) rather than an empty array — which is what a reader of a
/// hex dump needs to tell "no secret was exported" apart from "a zero-length one was".
///
/// Unlike [`POPULATED_BUNDLE_CBOR_HEX`] this was not frozen from a pre-newtype build, and it did
/// not need to be: no byte of it passes through the type this slice changed.
const EMPTY_BUNDLE_CBOR_HEX: &str = concat!(
    // map(6)
    "a6",
    "6776657273696f6e01",                     // "version" -> 1
    "716e6f64655f64656661756c745f64656b7380", // "node_default_deks" -> array(0)
    "6c657069736f64655f64656b7380",           // "episode_deks" -> array(0)
    "66636f6e666967f6",                       // "config" -> null
    "6664726166747380",                       // "drafts" -> array(0)
    "6d756e777261705f736563726574f6",         // "unwrap_secret" -> null  <-- the None case
);

#[test]
fn the_empty_bundle_encodes_to_the_frozen_bytes() {
    assert_eq!(
        hex::encode(to_cbor(&LocalState::empty())),
        EMPTY_BUNDLE_CBOR_HEX,
        "the empty CAIRNL1 bundle encoding moved — see this file's header before re-freezing"
    );
}

#[test]
fn the_frozen_empty_bytes_decode_to_the_empty_bundle() {
    let bytes = hex::decode(EMPTY_BUNDLE_CBOR_HEX).expect("the pin is valid hex");
    let back = from_cbor(&bytes).expect("an empty bundle must still parse");
    assert_eq!(back, LocalState::empty());
    assert!(
        back.unwrap_secret().is_none(),
        "an absent secret must decode as absent, not as a zero-length or all-zero key"
    );
}

/// The encode half. A change to `Secret32`'s `Serialize`, to a field's serde attributes, or to
/// the field ORDER (CBOR maps here are written in declaration order) all fail here.
#[test]
fn the_populated_bundle_encodes_to_the_frozen_bytes() {
    assert_eq!(
        hex::encode(to_cbor(&populated_bundle())),
        POPULATED_BUNDLE_CBOR_HEX,
        "the CAIRNL1 bundle encoding moved — every off-site .localstate export was written in \
         the old shape, and a restored clinic reads its custody out of one. Do not re-freeze \
         this constant to make the test green; see this file's header."
    );
}

/// The decode half, and the one that actually models a restore: bytes written by a PREVIOUS
/// build, parsed by this one. `Deserialize` and `Serialize` can move together and still be
/// wrong; this asserts against bytes neither of them produced.
#[test]
fn the_frozen_bytes_decode_to_the_populated_bundle() {
    let bytes = hex::decode(POPULATED_BUNDLE_CBOR_HEX).expect("the pin is valid hex");
    let back = from_cbor(&bytes).expect("a bundle written by the previous build must still parse");
    assert_eq!(
        back,
        populated_bundle(),
        "a bundle written by the previous build decoded to something else — an existing \
         off-site export would restore the wrong custody, or none"
    );
}

// ---------------------------------------------------------------------------------------
// Task 11 (#500): the export gains the actor registry — freeze FIRST, change SECOND
// ---------------------------------------------------------------------------------------
//
// The method is the same one #511 used above, applied to a genuinely new slot rather than a
// type change: freeze what the CURRENT (pre-registry) build produces, commit that pin on its
// own, and only THEN add `LocalState::actor_registry`. A pin taken after the field exists
// would prove nothing about whether the addition stayed additive.

/// A second custody row, distinct from [`wrapped_dek_placeholder`]'s so a fixture asking for
/// TWO rows (this file's pre-registry freeze does) cannot pass by accident if the code
/// collapsed them into one.
fn dek_cbor(n: u8) -> Vec<u8> {
    episode_dek_to_cbor(&EpisodeDek {
        event_id: format!("00000000-0000-0000-0000-00000000000{n}"),
        dek_wrapped: (0u8..8).map(|i| i.wrapping_mul(3).wrapping_add(n)).collect(),
    })
}

/// The exact CAIRNL1 CBOR a PRE-registry build produces, frozen 2026-09-06. This is a
/// DIFFERENT fixture from [`populated_bundle`] (two custody rows, a fresh secret lineage)
/// deliberately — it exists solely to pin the shape Task 11 must not disturb, so it should
/// not be entangled with the #511 pin's own reason for being.
///
/// Once `actor_registry` exists, no live call can reproduce these bytes any more — the new
/// field always appears in the encoding (see `the_empty_registry_encoding_is_pinned`'s doc for
/// why it must, rather than being skipped when empty). So after Task 11 lands, this constant's
/// job is exactly [`an_old_bundle_still_parses_with_the_registry_absent`]: proving an export an
/// OLD build wrote still restores under TODAY's code, registry defaulted to empty rather than
/// refused.
const PRE_REGISTRY_BUNDLE_HEX: &str = concat!(
    "a66776657273696f6e01716e6f64655f64656661756c745f64656b73806c6570",
    "69736f64655f64656b7382984518a21868186518761865186e1874185f186918",
    "641878182418301830183018301830183018301830182d183018301830183018",
    "2d1830183018301830182d1830183018301830182d1830183018301830183018",
    "30183018301830183018301831186b18641865186b185f187718721861187018",
    "701865186418880104070a0d101316984518a21868186518761865186e187418",
    "5f186918641878182418301830183018301830183018301830182d1830183018",
    "301830182d1830183018301830182d1830183018301830182d18301830183018",
    "3018301830183018301830183018301832186b18641865186b185f1877187218",
    "61187018701865186418880205080b0e11141766636f6e666967f66664726166",
    "7473806d756e777261705f7365637265749820091017181e1825182c1833183a",
    "18411848184f1856185d1864186b1872187918801887188e1895189c18a318aa",
    "18b118b818bf18c618cd18d418db18e2",
);

#[test]
fn the_pre_registry_bundle_bytes_are_unchanged() {
    let ls = LocalState::from_custody(vec![dek_cbor(1), dek_cbor(2)], Some(secret_fixture(9)));
    assert_eq!(
        hex::encode(to_cbor(&ls)),
        PRE_REGISTRY_BUNDLE_HEX,
        "the PRE-registry CAIRNL1 shape moved before Task 11 even touched it — capture the \
         printed actual value and re-run; do not hand-edit this constant"
    );
}
