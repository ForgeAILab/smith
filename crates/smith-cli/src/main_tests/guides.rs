//! Shipped guide commands must resolve against the production parsers.

use std::ffi::OsString;

use agent_runtime::ability::SkillSource;

fn cli_path_exists(token: &str) -> bool {
    let words = token.strip_prefix("smith ").unwrap().split_whitespace();
    let path = words
        .take_while(|word| !word.starts_with('-'))
        .take(2)
        .collect::<Vec<_>>();
    if path.is_empty() {
        return true;
    }
    if path.len() == 1
        && crate::cli::parse([OsString::from(path[0]), OsString::from("--help")]).is_ok()
    {
        return true;
    }
    // Required operands are immaterial to the command path. Probe the real
    // parser with a key/name, a provider flag, or a nested marketplace action.
    [
        vec![],
        vec!["guide-probe"],
        vec!["--provider", "guide-probe"],
        vec!["list"],
    ]
    .iter()
    .any(|suffix| crate::cli::parse(path.iter().chain(suffix).map(OsString::from)).is_ok())
}

fn slash_exists(token: &str) -> bool {
    let word = token.split_whitespace().next().unwrap_or_default();
    let name = word.strip_prefix('/').unwrap_or_default();
    !name.starts_with(|c: char| c.is_ascii_lowercase())
        || smith_client::commands::COMMANDS
            .iter()
            .any(|command| command.name == name)
}

#[test]
fn guide_commands_name_real_cli_paths_and_registered_slash_commands() {
    for guide in smith_runtime::built_in_skills::harness_guide_skills() {
        let SkillSource::Inline(body) = guide.source else {
            panic!("inline guide")
        };
        let mut in_fence = false;
        for line in body.lines() {
            if line.trim_start().starts_with("```") {
                in_fence = !in_fence;
                continue;
            }
            if in_fence {
                continue;
            }
            for token in line.split('`').skip(1).step_by(2) {
                if token.starts_with('/') {
                    assert!(
                        slash_exists(token),
                        "{} names unknown slash command `{token}`",
                        guide.name
                    );
                }
                if token.starts_with("smith ") {
                    assert!(
                        cli_path_exists(token),
                        "{} names unknown CLI path `{token}`",
                        guide.name
                    );
                }
            }
        }
    }
}

#[test]
fn guide_command_checks_reject_unknown_paths_and_allow_operands() {
    for valid in [
        "smith config",
        "smith models",
        "smith plugin",
        "smith sessions",
        "smith config explain",
        "smith config modules",
        "smith plugin add",
        "smith plugin marketplace",
        "smith sessions list",
        "smith setup credential",
        "smith setup",
        "smith -p",
    ] {
        assert!(cli_path_exists(valid), "{valid}");
    }
    for invalid in [
        "smith missing",
        "smith config missing",
        "smith plugin missing",
        "smith setup missing",
        "smith sessions missing",
    ] {
        assert!(!cli_path_exists(invalid), "{invalid}");
    }
    assert!(slash_exists("/model"));
    assert!(slash_exists("/think off"));
    assert!(!slash_exists("/missing-command"));
}
