use super::*;

fn s(x: &str) -> Option<String> {
    Some(x.to_string())
}

#[test]
fn reads_the_common_line() {
    let d = read_directions("500mg tds");
    assert_eq!(d.dose_amount, s("500"));
    assert_eq!(d.dose_unit, s("mg"));
    assert_eq!(d.frequency, s("three times daily"));
    assert!(d.unread.is_empty());
}

#[test]
fn reads_route_form_and_strength() {
    let d = read_directions("1g IV daily");
    assert_eq!((d.route, d.frequency), (s("iv"), s("daily")));
    let d = read_directions("tab 875/125mg 1 bd po");
    assert_eq!(d.form, s("tablet"));
    assert_eq!(d.strength, s("875/125 mg"));
    assert_eq!(d.dose_amount, s("1"));
    assert_eq!(d.dose_unit, None);
    assert_eq!(d.route, s("po"));
    assert_eq!(d.frequency, s("twice daily"));
    assert!(d.unread.is_empty());
}

#[test]
fn keeps_decimals_as_typed() {
    for (typed, amount) in [
        ("1.5mg", "1.5"),
        ("0.5 mg", "0.5"),
        (".5mg", ".5"),
        ("12.5 mg", "12.5"),
    ] {
        let d = read_directions(typed);
        assert_eq!(d.dose_amount, s(amount), "{typed}");
        assert_eq!(d.dose_unit, s("mg"), "{typed}");
    }
    let d = read_directions("1,5mg");
    assert_eq!(d.dose_amount, None);
    assert_eq!(d.dose_unit, None);
    assert_eq!(d.unread, vec!["1,5mg"]);
}

#[test]
fn prn_and_stat() {
    let d = read_directions("2 puffs inh prn");
    assert!(d.prn);
    assert_eq!((d.dose_unit, d.route), (s("puffs"), s("inh")));
    assert!(read_directions("1g iv stat").stat);
}

#[test]
fn two_doses_is_not_a_guess() {
    let d = read_directions("500mg 250mg tds");
    assert_eq!((d.dose_amount, d.dose_unit), (None, None));
    assert_eq!(d.unread, vec!["500 mg", "250 mg"]);
    assert_eq!(d.frequency, s("three times daily"));
}

#[test]
fn unknown_words_are_unread_never_dropped() {
    assert_eq!(
        read_directions("500mg tds with food").unread,
        vec!["with", "food"]
    );
}

#[test]
fn term_to_sign_keeps_what_was_typed_unless_unfinished() {
    assert_eq!(term_to_sign("amox", Some("amoxicillin")), "amoxicillin");
    assert_eq!(
        term_to_sign("Augmentin", Some("amoxicillin + clavulanic acid")),
        "Augmentin"
    );
    assert_eq!(
        term_to_sign("amoxycillin", Some("amoxicillin")),
        "amoxycillin"
    );
    assert_eq!(term_to_sign("x", None), "x");
}

#[test]
fn shown_name_adds_the_typed_brand_only_when_different() {
    assert_eq!(
        shown_name("amoxicillin + clavulanic acid", "Augmentin"),
        "amoxicillin + clavulanic acid (Augmentin)"
    );
    assert_eq!(shown_name("amoxicillin", "Amoxicillin"), "amoxicillin");
}

#[test]
fn two_of_a_kind_is_ambiguous_so_none_of_them_is_read() {
    let d = read_directions("bd tds");
    assert_eq!(d.frequency, None);
    assert_eq!(d.unread, vec!["bd", "tds"]);
    let d = read_directions("po iv");
    assert_eq!(d.route, None);
    assert_eq!(d.unread, vec!["po", "iv"]);
    let d = read_directions("tab cap");
    assert_eq!(d.form, None);
    assert_eq!(d.unread, vec!["tab", "cap"]);
}

#[test]
fn a_duration_or_interval_number_is_not_a_dose() {
    for typed in ["tds for 5 days", "every 8 hours", "2 weeks"] {
        let d = read_directions(typed);
        assert_eq!(d.dose_amount, None, "{typed}");
        assert!(
            d.unread
                .iter()
                .any(|u| u.chars().any(|c| c.is_ascii_digit())),
            "{typed}"
        );
    }
}

#[test]
fn a_second_strength_never_leaks_into_the_dose() {
    let d = read_directions("tab 500mg tab 250mg");
    assert_eq!((d.dose_amount, d.dose_unit), (None, None));
    assert_eq!(d.strength, s("500 mg"));
}

#[test]
fn a_strength_after_a_form_is_not_a_dose() {
    let d = read_directions("tab 500mg");
    assert_eq!(d.strength, s("500 mg"));
    assert_eq!(d.dose_amount, None);
}
