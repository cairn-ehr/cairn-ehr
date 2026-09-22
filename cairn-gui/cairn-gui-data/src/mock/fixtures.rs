//! The fixture patient set the `--mock` window browses.
//!
//! # Why there is more than one patient now
//!
//! The mock held exactly one patient while the window opened on a chart given by
//! `--patient <uuid>`. The funnel changes that: a clerk browses, sees a list, and picks —
//! and a candidate a clerk can see but cannot open is a dead end. So every fixture here has
//! to answer `demographics` as well as appear in a search.
//!
//! # These are chosen to exercise real shapes, not to look tidy
//!
//! Each row earns its place by being a case that has actually broken something, or by being
//! the case a rule was written for:
//!
//! | Fixture | What it is here to exercise |
//! |---|---|
//! | `Amina أمينة अमीना 阿明娜` | multi-script shaping and IME entry, for the operator accessibility pass on the reference shell |
//! | `Michaelowski, Samantha` | issue #636's headline case: `mich` must find her |
//! | `Fyodorowksi-Eschenbacher, Katarzyna` | a hyphenated compound found by EITHER half |
//! | `O'Brien-Smith, John` | apostrophe + hyphen, `SearchQuery::new`'s own worked case |
//! | `Wu, Mei` | a SHORT name — found by typing it whole, never gated away (#638) |
//! | a §5.4 John Doe callsign | an identity-pending chart, which is exactly the one a clerk needs when the family arrives with a name |

use cairn_patient_search::TrustState;
use uuid::Uuid;

/// One fixture patient — enough to be searched for, listed, and opened.
///
/// `sex` is a plain `String` because [`crate::port::Demographics`] carries one; it cannot
/// express *not recorded* distinctly from *recorded as this*, which is a principle-4 gap in
/// that struct rather than in this fixture. The registration path below writes
/// `"not recorded"` into it and means it literally.
#[derive(Debug, Clone)]
pub struct FixturePatient {
    /// A parsed `Uuid`, not a `String`, and that is a correctness choice rather than tidiness.
    /// While this was a string, a one-character typo in the table below made that patient
    /// silently invisible to every search — `as_candidate` parsed it, failed, and dropped the
    /// row — while `demographics` still answered for it. A fixture that cannot be found is
    /// exactly the duplicate-creating failure the funnel exists to prevent, so the parse
    /// happens once, in the private `patient` builder, where a typo is a loud panic at window
    /// start.
    pub uuid: Uuid,
    pub display_name: String,
    pub sex: String,
    /// ISO, and possibly reduced-precision — a registrar is frequently told only a year
    /// (principle 4), and `SearchQuery` carries that shape through unparsed. May also be the
    /// literal `"not recorded"`, for a registration that supplied no date at all: an EMPTY
    /// string here would render as a blank field, which reads as *not-yet-asked* or as a
    /// rendering bug rather than as "no date of birth was recorded" (principle 4 again — the
    /// same convention `sex` above follows).
    pub birth_date: String,
    pub identifiers: Vec<(String, String)>,
    pub trust: TrustState,
}

/// The id the single-patient mock has always used. Unchanged on purpose: the demographics
/// and note tabs' tests, and `commands.rs`'s fixture path, all name it.
pub const FIXTURE_UUID: &str = "00000000-0000-0000-0000-0000000000aa";

/// Build one fixture row. A free function rather than a `FixturePatient::new` so the table
/// below reads as a table.
fn patient(
    uuid: &str,
    display_name: &str,
    sex: &str,
    birth_date: &str,
    identifiers: &[(&str, &str)],
    trust: TrustState,
) -> FixturePatient {
    FixturePatient {
        // `expect` on purpose: every argument is a literal in the table below, so a failure
        // here is a typo a developer must see immediately, never a runtime condition.
        uuid: Uuid::parse_str(uuid).expect("a fixture uuid must be a valid UUID"),
        display_name: display_name.to_string(),
        sex: sex.to_string(),
        birth_date: birth_date.to_string(),
        identifiers: identifiers
            .iter()
            .map(|(s, v)| (s.to_string(), v.to_string()))
            .collect(),
        trust,
    }
}

/// The starting population of a `--mock` window.
pub fn starting_population() -> Vec<FixturePatient> {
    vec![
        patient(
            FIXTURE_UUID,
            // Latin / Arabic / Devanagari / Han in one label feeds the shaping and IME pass.
            // (Spike 0004 passed the shaping limb, I1, on the since-retired iced surface; its
            // IME limb I3 was never reached, so the obligation now sits with the operator
            // accessibility pass on the Tauri shell rather than with that spike.)
            "Amina أمينة अमीना 阿明娜",
            "female",
            "1984-03-02",
            &[("MRN", "12345"), ("National", "QLD-998877")],
            TrustState::Confirmed,
        ),
        patient(
            "00000000-0000-0000-0000-0000000000bb",
            "Michaelowski, Samantha",
            "female",
            "1979-11-20",
            &[("MRN", "20011")],
            TrustState::Confirmed,
        ),
        patient(
            "00000000-0000-0000-0000-0000000000cc",
            "Fyodorowksi-Eschenbacher, Katarzyna",
            "female",
            "1962-04-09",
            &[("MRN", "20012")],
            TrustState::Confirmed,
        ),
        patient(
            "00000000-0000-0000-0000-0000000000dd",
            "O'Brien-Smith, John",
            "male",
            "1990-01-31",
            &[("MRN", "20013")],
            TrustState::Confirmed,
        ),
        patient(
            "00000000-0000-0000-0000-0000000000ee",
            // Two characters. The prefix minimum gates PREFIXES, never short NAMES — a
            // clerk typing "Wu" whole must find her (#638's lesson, one layer up).
            "Wu, Mei",
            "female",
            "2001-07-14",
            &[("MRN", "20014")],
            TrustState::Confirmed,
        ),
        patient(
            "00000000-0000-0000-0000-0000000000ff",
            // A §5.4 callsign. Browsable and plainly identity-pending: hiding these would
            // manufacture a duplicate every time an unidentified patient is later named.
            "unknown-ed-site1-2026-07-03-00ab",
            "not recorded",
            "1975",
            &[],
            TrustState::Unconfirmed,
        ),
    ]
}
