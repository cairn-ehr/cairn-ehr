//! Reads a clinician's typed medication directions ("500mg tds", "1g IV daily") into
//! structured fields. Pure: no database, called by the window on every keystroke.
//!
//! The TYPED text is the signed record. This parse only fills a structured field when
//! it is certain; a wrong dose is a patient-safety defect, so every rule below leans
//! toward leaving a field `None` and putting the token in `unread`. Nothing is dropped.

/// Frequency abbreviations -> words. Source: ACSQHC, *Recommendations for terminology,
/// abbreviations and symbols used in the prescribing and administration of medicines*.
const FREQUENCIES: &[(&str, &str)] = &[
    ("mane", "every morning"),
    ("nocte", "at night"),
    ("daily", "daily"),
    ("bd", "twice daily"),
    ("tds", "three times daily"),
    ("qid", "four times daily"),
    ("q4h", "every 4 hours"),
    ("q6h", "every 6 hours"),
    ("q8h", "every 8 hours"),
    ("q12h", "every 12 hours"),
    ("weekly", "weekly"),
];

/// Routes -> open lower-case route token. Source: ACSQHC list (see FREQUENCIES).
const ROUTES: &[(&str, &str)] = &[
    ("po", "po"),
    ("oral", "po"),
    ("iv", "iv"),
    ("im", "im"),
    ("sc", "sc"),
    ("subcut", "sc"),
    ("sl", "sl"),
    ("pr", "pr"),
    ("top", "top"),
    ("inh", "inh"),
    ("neb", "neb"),
    ("pv", "pv"),
];

/// Dose units, as typed -> stored. Source: ACSQHC list (see FREQUENCIES).
const UNITS: &[(&str, &str)] = &[
    ("mg", "mg"),
    ("g", "g"),
    ("mcg", "mcg"),
    ("microgram", "microgram"),
    ("micrograms", "micrograms"),
    ("ml", "ml"),
    ("units", "units"),
    ("puff", "puff"),
    ("puffs", "puffs"),
    ("drop", "drop"),
    ("drops", "drops"),
    ("tablet", "tablet"),
    ("tablets", "tablets"),
    ("tab", "tab"),
    ("tabs", "tabs"),
    ("capsule", "capsule"),
    ("capsules", "capsules"),
    ("cap", "cap"),
    ("caps", "caps"),
];

/// Units that can describe a product strength ("875/125 mg"); counting units cannot.
const STRENGTH_UNITS: &[&str] = &["mg", "g", "mcg", "microgram", "micrograms", "ml", "units"];

/// Dose forms, as typed -> stored. Source: ACSQHC list (see FREQUENCIES).
const FORMS: &[(&str, &str)] = &[
    ("tab", "tablet"),
    ("tablet", "tablet"),
    ("cap", "capsule"),
    ("capsule", "capsule"),
    ("liquid", "liquid"),
    ("mixture", "mixture"),
    ("syrup", "syrup"),
    ("inj", "injection"),
    ("injection", "injection"),
    ("patch", "patch"),
    ("cream", "cream"),
    ("inhaler", "inhaler"),
];

/// What could be read with certainty; everything else is in `unread`, in typed order.
#[derive(Debug, Clone, PartialEq, Eq, Default, serde::Serialize)]
pub struct Directions {
    pub dose_amount: Option<String>,
    pub dose_unit: Option<String>,
    pub route: Option<String>,
    pub frequency: Option<String>,
    pub form: Option<String>,
    pub strength: Option<String>,
    pub prn: bool,
    pub stat: bool,
    pub unread: Vec<String>,
}

fn lookup(table: &[(&str, &'static str)], word: &str) -> Option<&'static str> {
    table.iter().find(|(k, _)| *k == word).map(|(_, v)| *v)
}

/// A plain decimal exactly as typed: digits with at most one '.', at least one digit.
/// No comma decimals: "1,5" is ambiguous across locales, so it is never read.
fn is_number(t: &str) -> bool {
    t.matches('.').count() <= 1
        && t.chars().any(|c| c.is_ascii_digit())
        && t.chars().all(|c| c.is_ascii_digit() || c == '.')
}

/// "875/125" style: two plain numbers. Only meaningful as a strength.
fn is_ratio(t: &str) -> bool {
    t.split_once('/')
        .is_some_and(|(a, b)| is_number(a) && is_number(b))
}

/// Lower-case, split on whitespace, and split a number glued to a unit ("500mg" ->
/// "500","mg"). A token that starts like a number but whose tail is not a pure word
/// ("1,5mg") stays whole, so it falls through to `unread` rather than half-reading as "1".
fn tokenise(text: &str) -> Vec<String> {
    let mut out = Vec::new();
    for raw in text.to_lowercase().split_whitespace() {
        let cut = raw.find(|c: char| !(c.is_ascii_digit() || c == '.' || c == '/'));
        match cut {
            Some(i) if i > 0 && raw[i..].chars().all(|c| c.is_ascii_alphabetic()) => {
                out.push(raw[..i].to_string());
                out.push(raw[i..].to_string());
            }
            _ => out.push(raw.to_string()),
        }
    }
    out
}

/// Sets `slot` only if empty; a second value is a contradiction, so it goes to unread.
fn fill(slot: &mut Option<String>, value: &str, token: &str, unread: &mut Vec<String>) {
    if slot.is_none() {
        *slot = Some(value.to_string());
    } else {
        unread.push(token.to_string());
    }
}

/// Read typed directions into fields; see the module comment for the "never guess" rule.
pub fn read_directions(text: &str) -> Directions {
    let toks = tokenise(text);
    let mut d = Directions::default();
    // Each dose candidate is (amount, unit); more than one means we cannot know which.
    let mut doses: Vec<(String, Option<String>)> = Vec::new();
    let mut i = 0;
    while i < toks.len() {
        let t = toks[i].as_str();
        let next = toks.get(i + 1).map(String::as_str);
        let unit = next.and_then(|n| lookup(UNITS, n));
        if t == "prn" {
            d.prn = true;
        } else if t == "stat" {
            d.stat = true;
        } else if let Some(w) = lookup(FREQUENCIES, t) {
            fill(&mut d.frequency, w, t, &mut d.unread);
        } else if let Some(r) = lookup(ROUTES, t) {
            fill(&mut d.route, r, t, &mut d.unread);
        } else if let Some(f) = lookup(FORMS, t) {
            fill(&mut d.form, f, t, &mut d.unread);
        } else if is_number(t) || is_ratio(t) {
            // A strength is a number right after a form word, with a strength unit ("tab 875/125 mg").
            let after_form = i > 0 && lookup(FORMS, &toks[i - 1]).is_some();
            let strength_unit = next.filter(|n| STRENGTH_UNITS.contains(n));
            if let (true, Some(u), None) = (after_form, strength_unit, &d.strength) {
                d.strength = Some(format!("{t} {u}"));
                i += 1;
            } else if is_ratio(t) {
                d.unread.push(t.to_string()); // a ratio is never a dose
            } else if let Some(u) = unit {
                doses.push((t.to_string(), Some(u.to_string())));
                i += 1;
            } else {
                doses.push((t.to_string(), None));
            }
        } else {
            d.unread.push(t.to_string());
        }
        i += 1;
    }
    if doses.len() == 1 {
        let (a, u) = doses.remove(0);
        (d.dose_amount, d.dose_unit) = (Some(a), u);
    } else {
        for (a, u) in doses {
            d.unread.push(match u {
                Some(u) => format!("{a} {u}"),
                None => a,
            });
        }
    }
    d
}

/// The text to sign as the drug term. If the clinician stopped typing part-way and then
/// picked a suggestion ("amox" -> amoxicillin), the typed text is unfinished: sign the
/// pick. Anything else they typed (a brand, a spelling) is their words and stays.
pub fn term_to_sign(typed: &str, picked_display: Option<&str>) -> String {
    let typed = typed.trim();
    match picked_display {
        Some(p) if p.to_lowercase().starts_with(&typed.to_lowercase()) => p.to_string(),
        _ => typed.to_string(),
    }
}

/// How a picked drug is shown: its listed name, plus what the clinician typed when that
/// differs ("amoxicillin + clavulanic acid (Augmentin)").
pub fn shown_name(display: &str, term: &str) -> String {
    let term = term.trim();
    if term.is_empty() || term.to_lowercase() == display.to_lowercase() {
        display.to_string()
    } else {
        format!("{display} ({term})")
    }
}

#[cfg(test)]
#[path = "line_tests.rs"]
mod tests;
