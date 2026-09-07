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
/// `LocalState::empty()` would pin almost nothing (six empty containers and a version byte),
/// and `unwrap_secret`'s encoding — the one field whose Rust type changes — would not appear in
/// it at all. Since Task 11 (#500) the registry slot is populated too, with a placeholder row
/// from the section below — see [`actor_row_cbor`].
fn populated_bundle() -> LocalState {
    let mut ls = LocalState::from_custody_and_registry(
        vec![episode_dek_to_cbor(&EpisodeDek {
            event_id: "00000000-0000-0000-0000-000000000001".to_string(),
            dek_wrapped: wrapped_dek_placeholder(),
        })],
        Some(secret_fixture(5)),
        vec![actor_row_cbor(7)],
    );
    ls.set_config(Some(b"config-blob".to_vec()));
    ls.set_drafts(vec![b"a draft".to_vec()]);
    ls
}

/// The exact CBOR TODAY's build produces for [`populated_bundle`].
///
/// Originally frozen 2026-09-04 (pre-#511, six fields); RE-FROZEN 2026-09-06 for Task 11
/// (#500), which added the `actor_registry` slot [`populated_bundle`] now also fills. This is
/// the "current shape" half of the pair the module header describes: [`PRE_REGISTRY_BUNDLE_HEX`]
/// below is the historical, NEVER-updated snapshot of what a build before this commit wrote (the
/// proof that the addition stayed additive); this constant tracks what a build AT OR AFTER this
/// commit writes, and moves — deliberately, reviewably — whenever the struct legitimately grows
/// again. See the module header before changing a single character of it by hand rather than by
/// re-running the test and pasting its printed value.
const POPULATED_BUNDLE_CBOR_HEX: &str = concat!(
    "a76776657273696f6e01716e6f64655f64656661756c745f64656b73806c6570",
    "69736f64655f64656b7381984518a21868186518761865186e1874185f186918",
    "641878182418301830183018301830183018301830182d183018301830183018",
    "2d1830183018301830182d1830183018301830182d1830183018301830183018",
    "30183018301830183018301831186b18641865186b185f187718721861187018",
    "70186518641888000306090c0f121566636f6e6669678b1863186f186e186618",
    "691867182d1862186c186f186266647261667473818718611820186418721861",
    "186618746d756e777261705f7365637265749820050c13181a18211828182f18",
    "36183d1844184b1852185918601867186e1875187c1883188a18911898189f18",
    "a618ad18b418bb18c218c918d018d718de6e6163746f725f7265676973747279",
    "8186070c1116181b1820",
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
/// not need to be: no byte of it passes through the type #511 changed. It DID need re-freezing
/// for Task 11 (#500), same as the populated pin, since `LocalState::empty()` now carries the
/// registry slot too (empty, but present — see the field's own doc for why it is never skipped).
const EMPTY_BUNDLE_CBOR_HEX: &str = concat!(
    "a76776657273696f6e01716e6f64655f64656661756c745f64656b73806c6570",
    "69736f64655f64656b738066636f6e666967f666647261667473806d756e7772",
    "61705f736563726574f66e6163746f725f726567697374727980",
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

/// The exact CAIRNL1 CBOR a PRE-registry build produces, frozen 2026-09-06 from
/// `LocalState::from_custody(vec![dek_cbor(1), dek_cbor(2)], Some(secret_fixture(9)))` — two
/// custody rows and a fresh secret lineage, a DIFFERENT fixture from [`populated_bundle`]'s
/// deliberately, so this pin does not get entangled with the #511 pin's own reason for being.
/// The `dek_cbor` helper that produced it did its one job and is gone (a helper with no
/// remaining caller is dead code the moment the constant it built is captured); the bytes
/// below are the record of what it once returned.
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

/// **What this test asserted BEFORE Task 11 added `actor_registry`, and why its body had to
/// change to keep meaning that.** Originally this called
/// `LocalState::from_custody(vec![dek_cbor(1), dek_cbor(2)], Some(secret_fixture(9)))` and
/// compared the live encoding to [`PRE_REGISTRY_BUNDLE_HEX`] for equality — that comparison
/// was run, was green, and THAT green run is what got frozen into the constant (see the
/// commit `test(#500): freeze the CAIRNL1 bytes...`). Once `actor_registry` exists, no live
/// encode can equal those bytes again: the field is never skipped (see
/// [`the_empty_registry_encoding_is_pinned`]'s doc), so it always adds one key to the map.
/// Asserting equality here would therefore either be permanently false (if the field really
/// always appears, as intended) or falsely true (only if a future change silently skips it
/// when empty — the exact mutant this whole file exists to catch).
///
/// So the assertion below is the INVERSE, and the test's OWN NAME says so — a fix-round
/// finding (Minor 6): a test still named `..._bytes_are_unchanged` while asserting
/// `assert_ne!` is exactly the stale-claim failure this file's own header warns readers to
/// watch for in comments, one level down in a test name instead. An old export, run through
/// today's `from_cbor` then `to_cbor`, must produce something OTHER than the old shape. If it
/// ever again produced exactly [`PRE_REGISTRY_BUNDLE_HEX`], that would mean `actor_registry`
/// had stopped travelling on the wire — silently, for a node with genuinely no actors,
/// indistinguishable from one whose registry never made it across. The decode half proper —
/// that the OLD bytes restore in the first place, with full custody intact — is
/// [`an_old_bundle_still_parses_with_the_registry_absent`], immediately below.
#[test]
fn re_encoding_a_pre_registry_bundle_never_reproduces_the_old_shape() {
    let old = hex::decode(PRE_REGISTRY_BUNDLE_HEX).expect("the pin is valid hex");
    let ls = from_cbor(&old).expect("a pre-registry export must still restore under today's code");
    let rewritten = hex::encode(to_cbor(&ls));
    assert_ne!(
        rewritten, PRE_REGISTRY_BUNDLE_HEX,
        "today's code reproduced the PRE-registry shape when writing an old bundle back out — \
         that means actor_registry is being silently omitted rather than always written"
    );
}

/// The decode half of the freeze above, and the one that survives past Task 11: an export
/// written by a build that predates `actor_registry` entirely must still restore, with the
/// slot defaulted to empty rather than the parse refused. `deny_unknown_fields` only refuses
/// an UNKNOWN key; a key this build KNOWS about that the OLD bytes simply never wrote is
/// exactly what `#[serde(default)]` exists to absorb (principle 11).
#[test]
fn an_old_bundle_still_parses_with_the_registry_absent() {
    let old = hex::decode(PRE_REGISTRY_BUNDLE_HEX).expect("the pin is valid hex");
    let ls = from_cbor(&old).expect("an export written before the registry existed must restore");
    assert!(
        ls.actor_registry().is_empty(),
        "absent registry = empty, never a parse failure"
    );
    // Anti-vacuity beyond the registry itself: the REST of a pre-registry export must land
    // intact too, or "still restores" would be true only for the one field this test names.
    assert_eq!(
        ls.episode_deks().len(),
        2,
        "the two custody rows must still be there"
    );
    assert!(
        ls.unwrap_secret().is_some(),
        "the custody secret must still be there"
    );
}

/// One actor-registry row's placeholder bytes, varying with `n` so two calls in the same
/// fixture do not collide. Short and content-free by design (same rationale as
/// [`wrapped_dek_placeholder`]): the shape of a `Vec<u8>` element is what a wire pin needs to
/// exercise, not a realistic `ActorRegistryRow` payload.
fn actor_row_cbor(n: u8) -> Vec<u8> {
    (0u8..6)
        .map(|i| i.wrapping_mul(5).wrapping_add(n))
        .collect()
}

/// The EMPTY registry's own encoding — #511's lesson, applied to this slot. A round-trip
/// test (encode then decode with the SAME code) cannot tell "the field is always emitted,
/// currently empty" apart from "the field is skipped when empty": both decode to
/// `actor_registry() == []`. Only a byte-level pin catches a future `skip_serializing_if`
/// that would make "the registry silently never travelled" indistinguishable from "this node
/// has no actors" — see [`an_old_bundle_still_parses_with_the_registry_absent`]'s pin for the
/// TRULY-absent case this one must stay visibly different from.
///
/// **These bytes are, in fact, byte-identical to [`EMPTY_BUNDLE_CBOR_HEX`]** — worth stating
/// so nobody reads that as a copy-paste mistake. `LocalState::empty()` and
/// `from_custody_and_registry(vec![], None, vec![])` are two different call sites landing on
/// the exact same all-empty value, which is the correct outcome: every field is empty/`None`
/// either way, so of course they encode identically. What THIS constant's own test protects
/// against is a FUTURE divergence — a `skip_serializing_if` added later would make
/// [`EMPTY_BUNDLE_CBOR_HEX`] shrink by one key while THIS one (if its own fixture ever grew a
/// reason to differ) would not, or vice versa; pinning both separately means either drifting
/// alone still reddens something.
const EMPTY_REGISTRY_BUNDLE_HEX: &str = concat!(
    "a76776657273696f6e01716e6f64655f64656661756c745f64656b73806c6570",
    "69736f64655f64656b738066636f6e666967f666647261667473806d756e7772",
    "61705f736563726574f66e6163746f725f726567697374727980",
);

#[test]
fn the_empty_registry_encoding_is_pinned() {
    let ls = LocalState::from_custody_and_registry(vec![], None, vec![]);
    assert_eq!(
        hex::encode(to_cbor(&ls)),
        EMPTY_REGISTRY_BUNDLE_HEX,
        "the empty-but-present registry encoding moved — see this test's doc before re-freezing"
    );
}

/// The non-empty case: a genuine round trip proves the slot actually carries content, which
/// [`the_empty_registry_encoding_is_pinned`] cannot (it has nothing to lose).
#[test]
fn a_registry_row_round_trips_through_the_seal() {
    let ls = LocalState::from_custody_and_registry(vec![], None, vec![actor_row_cbor(1)]);
    let back = from_cbor(&to_cbor(&ls)).unwrap();
    assert_eq!(back.actor_registry(), &[actor_row_cbor(1)]);
}
