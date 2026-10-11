use super::*;

use smith_config::trust::{Executable, ExecutableKind, TrustDecision, TrustStore};

#[test]
fn slash_commands_parses_list_trust_and_reload_with_the_skill_policy() {
    for (input, action) in [
        ("/commands", CommandsAction::List),
        (
            "/commands trust audit",
            CommandsAction::Trust("audit".into()),
        ),
        ("/commands reload", CommandsAction::Reload),
    ] {
        assert_eq!(
            parse(input),
            Ok(Command::Host(HostCommand::Commands(action)))
        );
    }
    assert!(
        parse("/commands trust")
            .unwrap_err()
            .contains("requires a command name")
    );
    for input in [
        "/commands audit",
        "/commands reload extra",
        "/commands trust audit extra",
    ] {
        assert!(parse(input).is_err(), "{input}");
    }
    let commands = COMMANDS
        .iter()
        .find(|spec| spec.name == "commands")
        .unwrap();
    let skills = COMMANDS.iter().find(|spec| spec.name == "skills").unwrap();
    assert_eq!(commands.requires_idle, skills.requires_idle);
    assert_eq!(commands.advanced, skills.advanced);
    assert_eq!(
        commands.complete_without_value,
        skills.complete_without_value
    );
    assert!(crate::file_commands::is_reserved_name("commands"));
}

#[test]
fn catalog_lookup_keeps_built_in_grammars_and_preserves_file_arguments() {
    let root = tempfile::tempdir().unwrap();
    std::fs::create_dir(root.path().join("commands")).unwrap();
    std::fs::write(
        root.path().join("commands/audit.md"),
        "---\ndescription: Examine changes\nargument-hint: <paths>\n---\nBody.",
    )
    .unwrap();
    for name in ["model", "commands"] {
        std::fs::write(
            root.path().join(format!("commands/{name}.md")),
            "File body.",
        )
        .unwrap();
    }
    let trust = TrustStore::open(root.path()).unwrap();
    let catalog = CommandCatalog::discover(Some(root.path()), None, &trust);
    assert_eq!(catalog.problems().len(), 2);
    let ParsedInput::BuiltIn(parsed) = parse_with("/model acme", &catalog).unwrap() else {
        panic!("built-in wins")
    };
    assert_eq!(
        parsed.command,
        Command::Ui(UiCommand::Model(Some("acme".into())))
    );
    assert_eq!(
        parse_with("/model extra value", &catalog).unwrap_err(),
        super::parse("/model extra value").unwrap_err()
    );
    assert!(matches!(
        parse_with("/commands reload", &catalog).unwrap(),
        ParsedInput::BuiltIn(_)
    ));
    let ParsedInput::File { name, arguments } =
        parse_with("  /audit \t first  path\nsecond path\n", &catalog).unwrap()
    else {
        panic!("file route")
    };
    assert_eq!(name, "audit");
    assert_eq!(arguments, "first  path\nsecond path\n");
    let ParsedInput::File { arguments, .. } = parse_with("/audit\n\t", &catalog).unwrap() else {
        panic!("file route")
    };
    assert!(arguments.is_empty());
    assert!(has_exact_name_with("/audit args", &catalog));
    assert!(has_exact_name_with("/model", &catalog));
    assert!(!has_exact_name_with("/aud", &catalog));
    assert_eq!(
        parse_with("/missing", &catalog).unwrap_err(),
        "unknown command `/missing` — type /help"
    );
    assert_eq!(
        parse_with("  /  ", &catalog).unwrap_err(),
        "select or enter a command"
    );
    assert!(!has_exact_name_with("", &catalog));
    let rows = matches_with("/aud", &catalog);
    assert_eq!(rows.len(), 1);
    assert_eq!(rows[0].name(), "audit");
    assert_eq!(rows[0].description(), "Examine changes");
    assert_eq!(rows[0].argument_hint(), Some("<paths>"));
    assert_eq!(rows[0].layer_label(), Some("user"));
    let builtin = matches_with("/model", &catalog)[0];
    assert_eq!(builtin.layer_label(), None);
    assert_eq!(builtin.argument_hint(), Some("[PROVIDER/MODEL]"));
    assert_eq!(matches_with("Examine", &catalog)[0].name(), "audit");
    let all = matches_with("/", &catalog);
    assert!(
        all[..COMMANDS.len()]
            .iter()
            .all(|row| matches!(row, MenuRow::BuiltIn(_)))
    );
    assert_eq!(all.last().unwrap().name(), "audit");
}

#[test]
fn file_prefixes_win_over_description_search_and_empty_catalog_matches_legacy() {
    let empty = CommandCatalog::empty();
    for query in ["/", "switch", "/rev", "/model args", "unknown"] {
        assert_eq!(
            matches_with(query, &empty)
                .iter()
                .map(MenuRow::name)
                .collect::<Vec<_>>(),
            matches(query)
                .iter()
                .map(|spec| spec.name)
                .collect::<Vec<_>>()
        );
    }
    let root = tempfile::tempdir().unwrap();
    std::fs::create_dir(root.path().join("commands")).unwrap();
    std::fs::write(
        root.path().join("commands/switch-audit.md"),
        "Examine changes.",
    )
    .unwrap();
    let trust = TrustStore::open(root.path()).unwrap();
    let catalog = CommandCatalog::discover(Some(root.path()), None, &trust);
    assert_eq!(
        matches_with("switch", &catalog)
            .iter()
            .map(MenuRow::name)
            .collect::<Vec<_>>(),
        ["switch-audit"]
    );
}

#[test]
fn menu_and_parser_include_withheld_project_winners_but_hide_shadowed_entries() {
    let root = tempfile::tempdir().unwrap();
    let project = tempfile::tempdir().unwrap();
    std::fs::create_dir(root.path().join("commands")).unwrap();
    std::fs::create_dir_all(project.path().join(".smith/commands")).unwrap();
    std::fs::write(root.path().join("commands/audit.md"), "User.").unwrap();
    let project_path = project.path().join(".smith/commands/audit.md");
    std::fs::write(&project_path, "Project.").unwrap();
    std::fs::write(
        project.path().join(".smith/commands/withheld.md"),
        "Awaiting approval.",
    )
    .unwrap();
    let mut trust = TrustStore::open(root.path()).unwrap();
    for trusted in [false, true] {
        if trusted {
            let executable =
                Executable::from_file(project.path(), ExecutableKind::SlashCommand, &project_path)
                    .unwrap();
            trust
                .record(project.path(), &executable, TrustDecision::Allow)
                .unwrap();
        }
        let catalog = CommandCatalog::discover(Some(root.path()), Some(project.path()), &trust);
        let rows = matches_with("/audit", &catalog);
        assert_eq!(rows.len(), 1);
        assert_eq!(
            rows[0].layer_label(),
            Some(if trusted { "project" } else { "user" })
        );
        let withheld = matches_with("/withheld", &catalog);
        assert_eq!(withheld.len(), 1);
        assert_eq!(withheld[0].layer_label(), Some("project"));
        let MenuRow::File(entry) = withheld[0] else {
            panic!("file row")
        };
        assert_eq!(entry.state, crate::file_commands::CommandState::NeedsTrust);
        assert!(has_exact_name_with("/withheld", &catalog));
        assert!(matches!(
            parse_with("/withheld multi\nline", &catalog).unwrap(),
            ParsedInput::File { .. }
        ));
    }
}

fn parse(input: &str) -> Result<Command, String> {
    super::parse(input).map(|parsed| parsed.command)
}

#[test]
fn status_completion_remains_a_complete_bare_command() {
    let status = COMMANDS
        .iter()
        .find(|command| command.name == "status")
        .unwrap();
    assert_eq!(completion(status), "/status");
    assert_eq!(
        parse(&completion(status)).unwrap(),
        Command::Host(HostCommand::Status)
    );
    assert_eq!(
        parse("/status --verbose").unwrap(),
        Command::Host(HostCommand::Diagnostics)
    );
    let model = COMMANDS
        .iter()
        .find(|command| command.name == "model")
        .unwrap();
    assert_eq!(completion(model), "/model ");
}

#[test]
fn help_and_completion_share_the_complete_registry() {
    let report = help();
    assert_eq!(
        report
            .getting_started
            .iter()
            .map(|command| command.name.as_str())
            .collect::<Vec<_>>(),
        ["model", "connect", "help"]
    );
    for command in &report.getting_started {
        let registered = COMMANDS
            .iter()
            .find(|registered| registered.name == command.name)
            .unwrap();
        assert_eq!(command.description, registered.description);
        assert!(command.argument_hint.is_empty());
    }
    let help = crate::help_report::render_plain(&report);
    assert!(help.starts_with("Getting started\n"));
    assert!(help.contains("/model — Switch model"));
    for command in COMMANDS {
        assert!(help.contains(&format!("/{}", command.name)), "{help}");
    }
    assert!(help.contains("Ctrl+R searches composer history"), "{help}");
    assert!(help.contains("Up/Down browse accepted"), "{help}");
    assert_eq!(
        matches("/rev")
            .into_iter()
            .map(|command| command.name)
            .collect::<Vec<_>>(),
        ["review", "revert"]
    );
}

#[test]
fn goal_argument_hint_uses_compact_alternatives_in_completion_and_help() {
    let goal = matches("/goal")[0];
    let expected = "[OBJECTIVE|edit …|budget N|pause|resume|clear]";
    assert_eq!(goal.argument_hint, expected);
    let guide = help();
    let help_goal = guide
        .primary
        .iter()
        .find(|command| command.name == "goal")
        .unwrap();
    assert_eq!(help_goal.argument_hint, expected);
}

#[test]
fn description_search_is_used_only_after_name_prefixes() {
    assert_eq!(
        matches("switch")
            .into_iter()
            .map(|command| command.name)
            .collect::<Vec<_>>(),
        ["model", "profile", "provider"]
    );
    assert_eq!(
        matches("/pro")
            .into_iter()
            .map(|command| command.name)
            .collect::<Vec<_>>(),
        ["profile", "provider"]
    );
    assert!(has_exact_name("/status --verbose"));
    assert!(!has_exact_name("/sta"));
}

#[test]
fn parser_returns_typed_actions_and_actionable_errors() {
    assert_eq!(
        parse("/model zai").expect("model"),
        Command::Ui(UiCommand::Model(Some("zai".into())))
    );
    assert_eq!(
        parse("/diff staged").expect("diff"),
        Command::Host(HostCommand::Diff(DiffScope::Git(Some("staged".into()))))
    );
    assert_eq!(
        parse("/context").expect("context"),
        Command::Host(HostCommand::Context)
    );
    assert_eq!(
        parse("/context 272k").expect("named context window"),
        Command::Ui(UiCommand::Context("272k".into()))
    );
    assert_eq!(
        parse("/context default").expect("default context window"),
        Command::Ui(UiCommand::Context("default".into()))
    );
    assert_eq!(
        parse("/model").expect("picker"),
        Command::Ui(UiCommand::Model(None))
    );
    assert_eq!(
        parse("/capabilities").expect("capability listing"),
        Command::Host(HostCommand::Capabilities(CapabilitiesAction::List))
    );
    assert_eq!(
        parse("/capabilities deny tool:shell").expect("session denial"),
        Command::Host(HostCommand::Capabilities(CapabilitiesAction::Deny(
            "tool:shell".into()
        )))
    );
    assert!(
        parse("/capabilities deny shell")
            .expect_err("not a pattern")
            .contains("<domain>:<name>")
    );
    assert_eq!(
        parse("/advisor").expect("advisor picker"),
        Command::Ui(UiCommand::Advisor(None))
    );
    assert_eq!(
        parse("/advisor acme/big").expect("advisor target"),
        Command::Ui(UiCommand::Advisor(Some("acme/big".into())))
    );
    assert_eq!(
        parse("/connect openrouter").expect("connection"),
        Command::Ui(UiCommand::Connect(Some("openrouter".into())))
    );
    assert_eq!(
        parse("/disconnect").expect("disconnect picker"),
        Command::Ui(UiCommand::Disconnect(None))
    );
    assert_eq!(
        parse("/think off").expect("thinking state"),
        Command::Ui(UiCommand::Think(Some("off".into())))
    );
    assert_eq!(
        parse("/effort high").expect("effort"),
        Command::Ui(UiCommand::Effort(Some("high".into())))
    );
    assert_eq!(
        parse("/think").expect("picker"),
        Command::Ui(UiCommand::Think(None))
    );
    assert_eq!(
        parse("/effort").expect("picker"),
        Command::Ui(UiCommand::Effort(None))
    );
    assert_eq!(
        parse("/agent resume child-7").expect("child resume"),
        Command::Confirm(ConfirmCommand::AgentResume("child-7".into()))
    );
    assert!(
        parse("/agent resume")
            .unwrap_err()
            .contains("requires a child ID")
    );
    assert!(parse("/missing").unwrap_err().contains("/help"));
}

#[test]
fn goal_parser_preserves_objectives_and_validates_controls() {
    assert_eq!(
        parse("/goal").unwrap(),
        Command::Host(HostCommand::Goal(GoalAction::Show))
    );
    assert_eq!(
        parse("/goal ship the persistent goal system").unwrap(),
        Command::Host(HostCommand::Goal(GoalAction::Create(
            "ship the persistent goal system".into()
        )))
    );
    assert_eq!(
        parse("/goal edit ship it safely").unwrap(),
        Command::Host(HostCommand::Goal(GoalAction::Edit("ship it safely".into())))
    );
    assert_eq!(
        parse("/goal budget 12000").unwrap(),
        Command::Host(HostCommand::Goal(GoalAction::Budget(Some(12_000))))
    );
    assert_eq!(
        parse("/goal budget none").unwrap(),
        Command::Host(HostCommand::Goal(GoalAction::Budget(None)))
    );
    assert_eq!(
        parse("/goal pause").unwrap(),
        Command::Host(HostCommand::Goal(GoalAction::Pause))
    );
    assert!(parse("/goal edit").unwrap_err().contains("objective"));
    assert!(parse("/goal budget 0").unwrap_err().contains("positive"));
}

#[test]
fn slash_skills_parses_its_list_and_trust_forms() {
    assert_eq!(
        parse("/skills"),
        Ok(Command::Host(HostCommand::Skills(SkillsAction::List)))
    );
    assert_eq!(
        parse("/skills trust deploy"),
        Ok(Command::Host(HostCommand::Skills(SkillsAction::Trust(
            "deploy".into()
        ))))
    );
    let error = parse("/skills trust").expect_err("a name is required");
    assert!(error.contains("requires a skill name"), "{error}");
    let error = parse("/skills deploy").expect_err("trust is the only verb");
    assert!(error.contains("is neither"), "{error}");
}

#[test]
fn the_mcp_command_parses_its_only_two_forms() {
    assert_eq!(
        parse("/mcp"),
        Ok(Command::Host(HostCommand::Mcp(McpAction::List)))
    );
    assert_eq!(
        parse("/mcp trust github"),
        Ok(Command::Host(HostCommand::Mcp(McpAction::Trust(
            "github".to_owned()
        ))))
    );
    assert!(parse("/mcp trust").is_err());
    assert!(parse("/mcp nonsense").is_err());
}

#[test]
fn every_entry_parses_its_own_usage_example() {
    let mut names = std::collections::BTreeSet::new();
    for spec in COMMANDS {
        assert!(names.insert(spec.name), "duplicate command {}", spec.name);
        let parsed = super::parse(spec.usage_example)
            .unwrap_or_else(|error| panic!("{}: {error}", spec.usage_example));
        assert_eq!(parsed.spec.name, spec.name, "{}", spec.usage_example);
        let completed = super::parse(&completion(spec)).expect("completion parses");
        assert_eq!(completed.spec.name, spec.name);
    }
}

#[test]
fn agent_and_diff_subarguments_are_routed_before_dispatch() {
    for (input, action) in [
        ("/agent", AgentAction::List),
        ("/agent parent", AgentAction::Parent),
        ("/agent next", AgentAction::Next),
        ("/agent previous", AgentAction::Previous),
        ("/agent child-7", AgentAction::Inspect("child-7".into())),
    ] {
        assert_eq!(
            parse(input).unwrap(),
            Command::Host(HostCommand::Agent(action))
        );
    }
    assert_eq!(
        parse("/diff last-turn").unwrap(),
        Command::Host(HostCommand::Diff(DiffScope::LastTurn))
    );
    for scope in [
        None,
        Some("all"),
        Some("staged"),
        Some("unstaged"),
        Some("untracked"),
        Some("commit:HEAD"),
        Some("base:main"),
        Some("file.txt"),
    ] {
        let input = scope.map_or_else(|| "/diff".to_owned(), |value| format!("/diff {value}"));
        assert_eq!(
            parse(&input).unwrap(),
            Command::Host(HostCommand::Diff(DiffScope::Git(scope.map(str::to_owned))))
        );
    }
    assert!(parse("/agent resume child-7 extra").is_err());
    assert!(parse("/agent next extra").is_err());
    assert!(parse("/diff staged extra").is_err());
    assert_eq!(
        parse("/accounts").unwrap_err(),
        "unknown command `/accounts` — type /help"
    );
}
