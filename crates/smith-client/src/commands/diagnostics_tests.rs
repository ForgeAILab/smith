use super::*;

fn parse(input: &str) -> Result<Command, String> {
    super::parse(input).map(|parsed| parsed.command)
}

#[test]
fn diagnostics_are_explicit_and_status_stays_concise() {
    assert_eq!(
        parse("/status").unwrap(),
        Command::Host(HostCommand::Status)
    );
    assert_eq!(
        parse("/status --verbose").unwrap(),
        Command::Host(HostCommand::Diagnostics)
    );
    assert_eq!(
        parse("/diagnostics").unwrap(),
        Command::Host(HostCommand::Diagnostics)
    );
    assert!(parse("/status nonsense").is_err());
}
