use super::{Command, HostCommand, ModulesAction, matches, parse};

#[test]
fn modules_has_one_discoverable_definition_and_a_typed_grammar() {
    assert_eq!(matches("modules").len(), 1);
    let parsed = parse("/modules").unwrap();
    assert_eq!(
        parsed.command,
        Command::Host(HostCommand::Modules(ModulesAction::List))
    );
    assert!(parsed.spec.complete_without_value);
    assert!(
        !parsed.spec.requires_idle,
        "listing remains available during a turn"
    );
    for (word, enabled) in [("on", true), ("off", false)] {
        assert_eq!(
            parse(&format!("/modules image-generation {word}"))
                .unwrap()
                .command,
            Command::Host(HostCommand::Modules(ModulesAction::Switch {
                id: "image-generation".into(),
                enabled
            }))
        );
    }
    for invalid in [
        "/modules image-generation",
        "/modules image-generation maybe",
        "/modules image-generation off extra",
    ] {
        assert!(parse(invalid).is_err(), "{invalid}");
    }
    assert!(
        parse("/modules image-generation maybe")
            .unwrap_err()
            .contains("/modules [ID on|off]")
    );
}
