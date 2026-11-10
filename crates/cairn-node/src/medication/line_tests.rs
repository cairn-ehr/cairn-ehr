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
    // A second frequency is a contradiction, not something to pick from.
    let d = read_directions("bd tds");
    assert_eq!(d.frequency, s("twice daily"));
    assert_eq!(d.unread, vec!["tds"]);
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
