//! The scenario table's contract: the two names parse, anything else is
//! a clear error — the harness never guesses a scenario.

use apple_planes::scenario::{NAMES, Scenario};

#[test]
fn the_known_scenarios_parse() {
    assert!(matches!(Scenario::parse("overlay"), Ok(Scenario::Overlay)));
    assert!(matches!(
        Scenario::parse("in-engine"),
        Ok(Scenario::InEngine)
    ));
}

#[test]
fn an_unknown_name_names_itself_in_the_error() {
    let err = Scenario::parse("transluscent").unwrap_err();
    assert!(err.contains("transluscent"), "{err}");
    assert!(err.contains(NAMES), "{err}");
}
