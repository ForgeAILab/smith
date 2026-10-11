use std::fs;

use smith_config::trust::TrustDecision;
use tempfile::TempDir;

use super::*;

/// A state root with `commands/<name>.md` written for each entry.
fn root_with(entries: &[(&str, &str)]) -> TempDir {
    let root = tempfile::tempdir().expect("a temporary root");
    for (name, body) in entries {
        write_command(root.path(), name, body);
    }
    root
}

fn write_command(root: &Path, name: &str, body: &str) -> PathBuf {
    let directory = root.join(COMMANDS_DIR);
    fs::create_dir_all(&directory).expect("a command directory");
    let path = directory.join(format!("{name}.md"));
    fs::write(&path, body).expect("a command body");
    path
}

const REVIEW: &str = "---\ndescription: Review Rust boundaries\n\
    argument-hint: <path>\n---\n\nRead the unsafe blocks first.\n";

#[test]
fn built_in_routes_win_even_if_a_catalog_contains_the_reserved_name() {
    let root = root_with(&[("audit", "Audit")]);
    let trust = TrustStore::open(root.path()).expect("trust store");
    let mut catalog = CommandCatalog::discover(Some(root.path()), None, &trust);
    catalog.entries[0].command.name = "model".to_owned();
    assert!(matches!(
        crate::commands::parse_with("/model", &catalog),
        Ok(crate::commands::ParsedInput::BuiltIn(_))
    ));
    let rows = crate::commands::matches_with("/model", &catalog);
    assert_eq!(rows.len(), 1);
    assert!(matches!(rows[0], crate::commands::MenuRow::BuiltIn(_)));
}

#[test]
fn empty_argument_hint_is_absent_and_quotes_match_skill_frontmatter() {
    let root = root_with(&[(
        "audit",
        "---\ndescription: \"text\"\nargument-hint: \n---\nBody.\n",
    )]);
    let found = discover_layer(root.path(), CommandLayer::User);
    assert!(found.problems.is_empty());
    assert_eq!(found.commands[0].description, "\"text\"");
    assert_eq!(found.commands[0].argument_hint, None);
}

#[test]
fn only_trusted_project_commands_shadow_user_commands() {
    let user = root_with(&[("audit", "User body.")]);
    let project = tempfile::tempdir().unwrap();
    let path = write_command(&project.path().join(".smith"), "audit", "Project body.");
    let mut trust = TrustStore::open(user.path()).unwrap();
    let executable =
        Executable::from_file(project.path(), ExecutableKind::SlashCommand, &path).unwrap();
    for (decision, expected) in [
        (None, CommandState::NeedsTrust),
        (Some(TrustDecision::Deny), CommandState::Denied),
        (Some(TrustDecision::Allow), CommandState::Runnable),
    ] {
        if let Some(decision) = decision {
            trust.record(project.path(), &executable, decision).unwrap();
        }
        let catalog = CommandCatalog::discover(Some(user.path()), Some(project.path()), &trust);
        assert!(catalog.problems().is_empty());
        assert_eq!(catalog.entries().len(), 2);
        assert_eq!(catalog.entries()[0].command.layer, CommandLayer::Project);
        assert_eq!(catalog.entries()[0].state, expected);
        let trusted = expected == CommandState::Runnable;
        assert_eq!(
            catalog.entries()[1].state,
            if trusted {
                CommandState::Shadowed
            } else {
                CommandState::Runnable
            }
        );
        let winner = catalog.resolve("audit").unwrap();
        assert_eq!(
            winner.command.layer,
            if trusted {
                CommandLayer::Project
            } else {
                CommandLayer::User
            }
        );
        assert_eq!(catalog.runnable().count(), 1);
        assert_eq!(
            catalog
                .prepare("audit", "", Some(project.path()), &trust)
                .unwrap()
                .prompt,
            if trusted {
                "Project body."
            } else {
                "User body."
            }
        );
    }
    fs::write(&path, "Changed project body.").unwrap();
    let catalog = CommandCatalog::discover(Some(user.path()), Some(project.path()), &trust);
    assert_eq!(catalog.entries()[0].state, CommandState::Changed);
    assert_eq!(catalog.entries()[1].state, CommandState::Runnable);
    assert_eq!(
        catalog.resolve("audit").unwrap().command.layer,
        CommandLayer::User
    );
}

#[test]
fn withheld_project_winners_are_listed_and_refused_with_an_approval_route() {
    let root = tempfile::tempdir().unwrap();
    let project = tempfile::tempdir().unwrap();
    let path = write_command(&project.path().join(".smith"), "audit", "Project body.");
    let mut trust = TrustStore::open(root.path()).unwrap();
    for (decision, expected) in [
        (None, CommandState::NeedsTrust),
        (Some(TrustDecision::Deny), CommandState::Denied),
    ] {
        if let Some(decision) = decision {
            let executable =
                Executable::from_file(project.path(), ExecutableKind::SlashCommand, &path).unwrap();
            trust.record(project.path(), &executable, decision).unwrap();
        }
        let catalog = CommandCatalog::discover(None, Some(project.path()), &trust);
        assert_eq!(catalog.resolve("audit").unwrap().state, expected);
        assert_eq!(catalog.runnable().count(), 0);
        let refusal = catalog
            .prepare("audit", "", Some(project.path()), &trust)
            .unwrap_err();
        assert_eq!(refusal.name, "audit");
        assert_eq!(refusal.reason, expected.reason("audit"));
        assert!(refusal.reason.contains("/commands trust audit"));
    }
}

#[test]
fn preparation_rechecks_project_content_even_without_reloading() {
    let user = root_with(&[("audit", "User body.")]);
    let project = tempfile::tempdir().unwrap();
    let path = write_command(&project.path().join(".smith"), "audit", "Project body.");
    let mut trust = TrustStore::open(user.path()).unwrap();
    let executable =
        Executable::from_file(project.path(), ExecutableKind::SlashCommand, &path).unwrap();
    trust
        .record(project.path(), &executable, TrustDecision::Allow)
        .unwrap();
    let catalog = CommandCatalog::discover(Some(user.path()), Some(project.path()), &trust);
    fs::write(&path, "Changed body.").unwrap();
    let refusal = catalog
        .prepare("audit", "", Some(project.path()), &trust)
        .unwrap_err();
    assert_eq!(refusal.reason, CommandState::Changed.reason("audit"));
    let rebuilt = CommandCatalog::discover(None, Some(project.path()), &trust);
    assert_eq!(
        rebuilt.resolve("audit").unwrap().state,
        CommandState::Changed
    );
    assert_eq!(rebuilt.runnable().count(), 0);
    assert!(contribution_modules(&rebuilt).is_empty());
    assert_eq!(
        rebuilt
            .prepare("audit", "", Some(project.path()), &trust)
            .unwrap_err()
            .reason,
        refusal.reason
    );
    trust.forget(project.path()).unwrap();
    assert_eq!(
        catalog
            .prepare("audit", "", Some(project.path()), &trust)
            .unwrap_err()
            .reason,
        CommandState::NeedsTrust.reason("audit")
    );
    // Reload is the admission boundary for names and metadata; preparation
    // still consults current decisions for the selected project file.
    let current =
        Executable::from_file(project.path(), ExecutableKind::SlashCommand, &path).unwrap();
    trust
        .record(project.path(), &current, TrustDecision::Allow)
        .unwrap();
    assert_eq!(
        catalog
            .prepare("audit", "", Some(project.path()), &trust)
            .unwrap()
            .prompt,
        "Changed body."
    );
    trust
        .record(project.path(), &current, TrustDecision::Deny)
        .unwrap();
    assert_eq!(
        catalog
            .prepare("audit", "", Some(project.path()), &trust)
            .unwrap_err()
            .reason,
        CommandState::Denied.reason("audit")
    );
}

#[test]
fn project_admission_checks_the_digest_of_the_bytes_already_read() {
    let root = tempfile::tempdir().unwrap();
    let project = tempfile::tempdir().unwrap();
    let path = write_command(&project.path().join(".smith"), "audit", "Earlier body.");
    let loaded = read_command(&path).unwrap();
    fs::write(&path, "Current body.").unwrap();
    let mut trust = TrustStore::open(root.path()).unwrap();
    let executable =
        Executable::from_file(project.path(), ExecutableKind::SlashCommand, &path).unwrap();
    trust
        .record(project.path(), &executable, TrustDecision::Allow)
        .unwrap();
    assert_eq!(
        project_state(project.path(), &trust, &path, &loaded.content).unwrap(),
        CommandState::Changed
    );
}

#[test]
fn user_preparation_uses_edits_trims_the_prompt_and_refuses_broken_files() {
    let user = root_with(&[("audit", "Review $ARGUMENTS for bugs.\n")]);
    let trust = TrustStore::open(user.path()).unwrap();
    let catalog = CommandCatalog::discover(Some(user.path()), None, &trust);
    let prepared = catalog
        .prepare("audit", "src/lib.rs", None, &trust)
        .unwrap();
    assert_eq!(prepared.prompt, "Review src/lib.rs for bugs.");
    assert_eq!(prepared.name, "audit");
    assert_eq!(prepared.layer, CommandLayer::User);
    let path = &catalog.resolve("audit").unwrap().command.path;
    fs::write(path, "\n\nEdited $ARGUMENTS.\n\n").unwrap();
    assert_eq!(
        catalog
            .prepare("audit", "src/lib.rs", None, &trust)
            .unwrap()
            .prompt,
        "Edited src/lib.rs."
    );
    fs::write(path, "\n\nNo placeholder.\n").unwrap();
    assert_eq!(
        catalog
            .prepare("audit", "extra", None, &trust)
            .unwrap()
            .prompt,
        "No placeholder.\n\nextra"
    );
    fs::write(path, "---\nunterminated\n").unwrap();
    assert!(
        catalog
            .prepare("audit", "", None, &trust)
            .unwrap_err()
            .reason
            .contains("frontmatter")
    );
    fs::remove_file(path).unwrap();
    assert!(
        catalog
            .prepare("audit", "", None, &trust)
            .unwrap_err()
            .reason
            .contains("cannot be read")
    );
    assert!(catalog.prepare("missing", "", None, &trust).is_err());
}

#[test]
fn catalog_discovery_reports_bad_files_and_orders_names_without_creating_directories() {
    let root = tempfile::tempdir().unwrap();
    let project = tempfile::tempdir().unwrap();
    let trust = TrustStore::open(root.path()).unwrap();
    let empty = CommandCatalog::discover(Some(root.path()), Some(project.path()), &trust);
    assert!(empty.entries().is_empty());
    assert!(empty.problems().is_empty());
    assert!(!root.path().join("commands").exists());
    assert!(!project.path().join(".smith").exists());
    assert!(!trust.path().exists());
    write_command(root.path(), "zebra", "Body.");
    write_command(root.path(), "alpha", "Body.");
    write_command(root.path(), "broken", "---\nno delimiter");
    let catalog = CommandCatalog::discover(Some(root.path()), Some(project.path()), &trust);
    assert_eq!(
        catalog
            .entries()
            .iter()
            .map(|entry| entry.command.name.as_str())
            .collect::<Vec<_>>(),
        ["alpha", "zebra"]
    );
    assert_eq!(catalog.problems()[0].name, "broken");
    assert!(CommandCatalog::empty().resolve("alpha").is_none());
}

#[cfg(unix)]
#[test]
fn project_symlink_escape_is_excluded_and_refused_after_discovery() {
    use std::os::unix::fs::symlink;
    let user = root_with(&[("audit", "Same body.")]);
    let project = tempfile::tempdir().unwrap();
    let path = write_command(&project.path().join(".smith"), "audit", "Same body.");
    let mut trust = TrustStore::open(user.path()).unwrap();
    let executable =
        Executable::from_file(project.path(), ExecutableKind::SlashCommand, &path).unwrap();
    trust
        .record(project.path(), &executable, TrustDecision::Allow)
        .unwrap();
    let catalog = CommandCatalog::discover(Some(user.path()), Some(project.path()), &trust);
    fs::remove_file(&path).unwrap();
    symlink(user.path().join("commands/audit.md"), &path).unwrap();
    assert!(
        catalog
            .prepare("audit", "", Some(project.path()), &trust)
            .unwrap_err()
            .reason
            .contains("outside the project")
    );
    let rebuilt = CommandCatalog::discover(Some(user.path()), Some(project.path()), &trust);
    assert_eq!(rebuilt.entries().len(), 1);
    assert_eq!(
        rebuilt.resolve("audit").unwrap().command.layer,
        CommandLayer::User
    );
    assert_eq!(rebuilt.problems()[0].name, "audit");
    assert!(rebuilt.problems()[0].reason.contains("outside the project"));
}

#[test]
fn command_contributions_record_only_runnable_winners_and_grant_nothing() {
    let user = root_with(&[("audit", "User body.")]);
    let project = tempfile::tempdir().unwrap();
    let path = write_command(&project.path().join(".smith"), "audit", "Project body.");
    write_command(&project.path().join(".smith"), "withheld", "Project body.");
    let mut trust = TrustStore::open(user.path()).unwrap();
    let catalog = CommandCatalog::discover(Some(user.path()), Some(project.path()), &trust);
    let records = contribution_modules(&catalog);
    assert_eq!(records.len(), 1);
    let record = &records[0];
    assert_eq!(record.id.as_str(), "commands/user/audit");
    assert_eq!(
        record.revision.as_str(),
        ContentDigest::of(b"User body.").as_hex()
    );
    assert_eq!(
        record.provenance,
        ModuleProvenance::UserManifest(format!(
            "user:{}",
            user.path().join("commands/audit.md").display()
        ))
    );
    assert_eq!(record.trust, ModuleTrust::ContentOnly);
    assert_eq!(
        record.contributions,
        [Contribution::Command {
            name: "audit".into()
        }]
    );
    assert!(record.requested_capabilities.is_empty());
    assert!(record.granted_capabilities.is_empty());
    let executable =
        Executable::from_file(project.path(), ExecutableKind::SlashCommand, &path).unwrap();
    trust
        .record(project.path(), &executable, TrustDecision::Allow)
        .unwrap();
    let catalog = CommandCatalog::discover(Some(user.path()), Some(project.path()), &trust);
    let records = contribution_modules(&catalog);
    assert_eq!(records.len(), 1);
    assert_eq!(records[0].id.as_str(), "commands/project/audit");
    assert_eq!(
        records[0].provenance,
        ModuleProvenance::UserManifest(format!("project:{}", path.display()))
    );
    assert!(records[0].requested_capabilities.is_empty());
    assert!(records[0].granted_capabilities.is_empty());
}

#[test]
fn file_stem_names_the_command_and_metadata_is_indexed() {
    let root = root_with(&[("rust-review", REVIEW)]);
    let found = discover_layer(root.path(), CommandLayer::User);
    assert!(found.problems.is_empty(), "{:?}", found.problems);
    assert_eq!(found.commands.len(), 1);
    let command = &found.commands[0];
    assert_eq!(command.name, "rust-review");
    assert_eq!(command.layer, CommandLayer::User);
    assert_eq!(command.description, "Review Rust boundaries");
    assert_eq!(command.argument_hint.as_deref(), Some("<path>"));
    assert_eq!(command.path, root.path().join("commands/rust-review.md"));
    assert_eq!(command.content, ContentDigest::of(REVIEW.as_bytes()));
}

#[test]
fn user_and_project_roots_have_the_same_fixed_layout() {
    let user = root_with(&[("user-only", REVIEW)]);
    let project = tempfile::tempdir().expect("a project");
    let project_root = project.path().join(".smith");
    write_command(&project_root, "project-only", REVIEW);
    let user_found = discover_layer(user.path(), CommandLayer::User);
    let project_found = discover_layer(&project_root, CommandLayer::Project);
    assert_eq!(user_found.commands[0].name, "user-only");
    assert_eq!(project_found.commands[0].name, "project-only");
    assert_eq!(project_found.commands[0].layer, CommandLayer::Project);
    assert_eq!(CommandLayer::User.label(), "user");
    assert_eq!(CommandLayer::Project.label(), "project");
}

#[test]
fn frontmatter_name_mismatch_is_named_and_registers_nothing() {
    let root = root_with(&[(
        "rust-review",
        "---\nname: deploy\ndescription: Review\n---\nBody.\n",
    )]);
    let found = discover_layer(root.path(), CommandLayer::Project);
    assert!(found.commands.is_empty());
    assert_eq!(found.problems.len(), 1);
    assert_eq!(found.problems[0].name, "rust-review");
    assert_eq!(found.problems[0].layer, CommandLayer::Project);
    assert_eq!(
        found.problems[0].path,
        root.path().join("commands/rust-review.md")
    );
    assert!(found.problems[0].reason.contains("`deploy`"));
}

#[test]
fn matching_name_and_unknown_frontmatter_keys_are_accepted() {
    let root = root_with(&[(
        "ported",
        "---\nname: ported\nlicense: MIT\nallowed-tools: Read, Edit\n---\nBody.\n",
    )]);
    let found = discover_layer(root.path(), CommandLayer::User);
    assert!(found.problems.is_empty(), "{:?}", found.problems);
    assert_eq!(found.commands.len(), 1);
    assert_eq!(found.commands[0].description, "Body.");
    assert_eq!(found.commands[0].argument_hint, None);
}

#[test]
fn optional_or_empty_description_falls_back_to_the_body() {
    let root = root_with(&[
        ("bare", "\n  ## First body line  \nSecond line.\n"),
        (
            "missing",
            "---\nargument-hint: <path>\n---\n\nFirst body line\n",
        ),
        ("empty", "---\ndescription: \n---\nFirst body line\n"),
        ("markers", "###\n"),
    ]);
    let found = discover_layer(root.path(), CommandLayer::User);
    assert!(found.problems.is_empty(), "{:?}", found.problems);
    assert_eq!(found.commands.len(), 4);
    for command in &found.commands {
        assert_eq!(
            command.description,
            if command.name == "markers" {
                ""
            } else {
                "First body line"
            }
        );
    }
    let bare = read_command(&root.path().join("commands/bare.md")).expect("a bare command");
    assert_eq!(bare.body, "\n  ## First body line  \nSecond line.\n");
}

#[test]
fn fallback_description_is_truncated_by_characters() {
    let root = root_with(&[("wordy", &"é".repeat(MAX_DESCRIPTION_CHARS + 1))]);
    let found = discover_layer(root.path(), CommandLayer::User);
    assert!(found.problems.is_empty(), "{:?}", found.problems);
    assert_eq!(
        found.commands[0].description,
        "é".repeat(MAX_DESCRIPTION_CHARS)
    );
}

#[test]
fn explicit_metadata_bounds_accept_the_limit_and_refuse_excess() {
    let root = tempfile::tempdir().expect("a temporary root");
    let description = "é".repeat(MAX_DESCRIPTION_CHARS);
    let hint = "é".repeat(MAX_ARGUMENT_HINT_CHARS);
    let path = write_command(
        root.path(),
        "bounded",
        &format!("---\ndescription: {description}\nargument-hint: {hint}\n---\nBody.\n"),
    );
    let loaded = read_command(&path).expect("metadata at the limits");
    assert_eq!(loaded.description, description);
    assert_eq!(loaded.argument_hint.as_deref(), Some(hint.as_str()));
    for (name, key, limit) in [
        ("wordy", "description", MAX_DESCRIPTION_CHARS),
        ("long-hint", "argument-hint", MAX_ARGUMENT_HINT_CHARS),
    ] {
        write_command(
            root.path(),
            name,
            &format!("---\n{key}: {}\n---\nBody.\n", "é".repeat(limit + 1)),
        );
    }
    let found = discover_layer(root.path(), CommandLayer::User);
    assert_eq!(found.commands.len(), 1);
    assert_eq!(found.problems.len(), 2);
    assert_eq!(found.problems[0].name, "long-hint");
    assert!(found.problems[0].reason.contains("`argument-hint`"));
    assert_eq!(found.problems[1].name, "wordy");
    assert!(found.problems[1].reason.contains("`description`"));
}

#[test]
fn malformed_frontmatter_does_not_stop_other_commands() {
    let root = root_with(&[
        ("good", REVIEW),
        ("unclosed", "---\ndescription: Missing end\n"),
        ("also-good", REVIEW),
        ("bad-line", "---\nnot a field\n---\nBody.\n"),
        ("just-delimiter", "---"),
    ]);
    let found = discover_layer(root.path(), CommandLayer::User);
    assert_eq!(
        found
            .commands
            .iter()
            .map(|command| command.name.as_str())
            .collect::<Vec<_>>(),
        ["also-good", "good"]
    );
    assert_eq!(found.problems.len(), 3);
    assert_eq!(found.problems[0].name, "bad-line");
    assert!(found.problems[0].reason.contains("not `key: value`"));
    assert_eq!(found.problems[1].name, "just-delimiter");
    assert_eq!(found.problems[2].name, "unclosed");
    assert!(found.problems[2].reason.contains("not closed"));
}

#[test]
fn frontmatter_line_bound_includes_the_closing_delimiter() {
    let root = tempfile::tempdir().expect("a temporary root");
    let path = write_command(
        root.path(),
        "at-limit",
        &format!(
            "---\n{}---\nBody.\n",
            "unknown: ignored\n".repeat(MAX_FRONT_MATTER_LINES - 1)
        ),
    );
    assert!(read_command(&path).is_ok());
    write_command(
        root.path(),
        "over-limit",
        &format!(
            "---\n{}---\nBody.\n",
            "unknown: ignored\n".repeat(MAX_FRONT_MATTER_LINES)
        ),
    );
    let found = discover_layer(root.path(), CommandLayer::User);
    assert_eq!(found.commands.len(), 1);
    assert_eq!(found.problems.len(), 1);
    assert_eq!(found.problems[0].name, "over-limit");
    assert!(found.problems[0].reason.contains("within 64 lines"));
}

#[test]
fn crlf_frontmatter_preserves_the_body() {
    let root = root_with(&[("crlf", "---\r\ndescription: Review\r\n---\r\n\r\nBody.\r\n")]);
    let loaded = read_command(&root.path().join("commands/crlf.md")).expect("CRLF metadata");
    assert_eq!(loaded.description, "Review");
    assert_eq!(loaded.body, "\r\nBody.\r\n");
}

#[test]
fn empty_bodies_with_or_without_frontmatter_are_problems() {
    let root = root_with(&[
        ("bare-empty", "\n   \n"),
        ("empty", "---\ndescription: Does nothing\n---\n\n   \n"),
    ]);
    let found = discover_layer(root.path(), CommandLayer::User);
    assert!(found.commands.is_empty());
    assert_eq!(found.problems.len(), 2);
    assert!(
        found
            .problems
            .iter()
            .all(|problem| problem.reason.contains("no prompt body"))
    );
}

#[test]
fn name_grammar_accepts_only_one_to_64_ascii_letters_digits_and_hyphens() {
    for name in ["a", "0", "-", "rust-review-2", &"a".repeat(64)] {
        assert!(validate_name(name).is_ok(), "{name}");
    }
    for name in [
        "",
        "My Command",
        "Upper",
        "under_score",
        "with.dot",
        "é",
        "a/b",
        &"a".repeat(65),
    ] {
        assert!(validate_name(name).is_err(), "{name}");
    }
    let root = root_with(&[("My Command", REVIEW), ("good", REVIEW)]);
    let found = discover_layer(root.path(), CommandLayer::User);
    assert_eq!(found.commands.len(), 1);
    assert_eq!(found.problems.len(), 1);
    assert_eq!(found.problems[0].name, "My Command");
    assert!(found.problems[0].reason.contains("ASCII"));
}

#[test]
fn every_builtin_name_is_reserved_and_collisions_are_named() {
    for command in crate::commands::COMMANDS {
        assert!(is_reserved_name(command.name), "{}", command.name);
    }
    assert!(!is_reserved_name("model-extra"));
    assert!(!is_reserved_name("MODEL"));
    let root = root_with(&[("quit", REVIEW), ("model", REVIEW)]);
    let found = discover_layer(root.path(), CommandLayer::User);
    assert!(found.commands.is_empty());
    assert_eq!(found.problems.len(), 2);
    assert_eq!(found.problems[0].name, "model");
    assert_eq!(found.problems[1].name, "quit");
    assert!(
        found
            .problems
            .iter()
            .all(|problem| problem.reason.contains("reserved"))
    );
}

#[test]
fn file_size_bound_accepts_the_limit_and_reports_excess_without_truncation() {
    let root = tempfile::tempdir().expect("a temporary root");
    let limit = usize::try_from(MAX_BODY_BYTES).expect("a usize");
    let path = write_command(root.path(), "at-limit", &"x".repeat(limit));
    let loaded = read_command(&path).expect("a file at the byte limit");
    assert_eq!(loaded.body.len(), limit);
    write_command(root.path(), "large", &"x".repeat(limit + 1));
    let found = discover_layer(root.path(), CommandLayer::User);
    assert_eq!(found.commands.len(), 1);
    assert_eq!(found.problems.len(), 1);
    assert_eq!(found.problems[0].name, "large");
    assert!(found.problems[0].reason.contains("65537 bytes"));
    assert!(found.problems[0].reason.contains("65536-byte limit"));
}

#[test]
fn non_utf8_content_is_named_and_other_files_still_load() {
    let root = root_with(&[("good", REVIEW)]);
    fs::write(root.path().join("commands/binary.md"), [0xff, 0xfe]).expect("non-UTF-8 bytes");
    let found = discover_layer(root.path(), CommandLayer::User);
    assert_eq!(found.commands.len(), 1);
    assert_eq!(found.problems.len(), 1);
    assert_eq!(found.problems[0].name, "binary");
    assert!(found.problems[0].reason.contains("not UTF-8"));
}

#[test]
fn count_bound_keeps_sorted_candidates_and_names_every_exclusion() {
    let root = tempfile::tempdir().expect("a temporary root");
    for index in (0..MAX_COMMANDS_PER_LAYER + 3).rev() {
        write_command(root.path(), &format!("command-{index:04}"), REVIEW);
    }
    let found = discover_layer(root.path(), CommandLayer::User);
    assert_eq!(found.commands.len(), MAX_COMMANDS_PER_LAYER);
    assert_eq!(found.commands[0].name, "command-0000");
    assert_eq!(
        found.commands[MAX_COMMANDS_PER_LAYER - 1].name,
        "command-0255"
    );
    assert_eq!(found.problems.len(), 3);
    for (index, problem) in found.problems.iter().enumerate() {
        assert_eq!(
            problem.name,
            format!("command-{:04}", MAX_COMMANDS_PER_LAYER + index)
        );
        assert!(problem.reason.contains("first 256"));
        assert!(problem.reason.contains("beyond the limit"));
    }
}

#[test]
fn discovery_sorts_by_stem_rather_than_the_md_suffix() {
    let root = root_with(&[("a-long", REVIEW), ("a", REVIEW), ("a-", REVIEW)]);
    let found = discover_layer(root.path(), CommandLayer::User);
    assert!(found.problems.is_empty(), "{:?}", found.problems);
    assert_eq!(
        found
            .commands
            .iter()
            .map(|command| command.name.as_str())
            .collect::<Vec<_>>(),
        ["a", "a-", "a-long"]
    );
}

#[test]
fn malformed_candidates_still_count_toward_the_read_bound() {
    let root = tempfile::tempdir().expect("a temporary root");
    write_command(root.path(), "a-malformed", "---\n");
    for index in 0..MAX_COMMANDS_PER_LAYER {
        write_command(root.path(), &format!("command-{index:04}"), REVIEW);
    }
    let found = discover_layer(root.path(), CommandLayer::User);
    assert_eq!(found.commands.len(), MAX_COMMANDS_PER_LAYER - 1);
    assert_eq!(found.problems.len(), 2);
    assert_eq!(found.problems[0].name, "a-malformed");
    assert_eq!(found.problems[1].name, "command-0255");
    assert!(found.problems[1].reason.contains("beyond the limit"));
}

#[test]
fn a_dot_md_file_is_reported_as_an_invalid_name() {
    let root = root_with(&[("good", REVIEW)]);
    fs::write(root.path().join("commands/.md"), REVIEW).expect("an invalid command file");
    let found = discover_layer(root.path(), CommandLayer::User);
    assert_eq!(found.commands.len(), 1);
    assert_eq!(found.problems.len(), 1);
    assert_eq!(found.problems[0].name, ".md");
    assert!(found.problems[0].reason.contains("ASCII"));
}

#[test]
fn subdirectories_and_non_md_files_are_ignored_silently() {
    let root = root_with(&[("real", REVIEW)]);
    let directory = root.path().join(COMMANDS_DIR);
    for name in ["notes.txt", "backup.md~", "UPPER.MD", "LICENSE"] {
        fs::write(directory.join(name), "not a command").expect("a loose file");
    }
    fs::create_dir_all(directory.join("nested.md")).expect("a subdirectory");
    fs::write(directory.join("nested.md/hidden.md"), REVIEW).expect("a nested command");
    let found = discover_layer(root.path(), CommandLayer::User);
    assert_eq!(found.commands.len(), 1);
    assert_eq!(found.commands[0].name, "real");
    assert!(found.problems.is_empty(), "{:?}", found.problems);
}

#[test]
fn missing_command_directories_and_roots_create_nothing() {
    let root = tempfile::tempdir().expect("a temporary root");
    for (path, layer) in [
        (root.path().to_path_buf(), CommandLayer::User),
        (root.path().join(".smith"), CommandLayer::Project),
    ] {
        let found = discover_layer(&path, layer);
        assert!(found.commands.is_empty());
        assert!(found.problems.is_empty());
        assert!(!path.join(COMMANDS_DIR).exists());
    }
    assert!(!root.path().join(".smith").exists());
}

#[test]
fn rereading_uses_current_bytes_and_digest_including_frontmatter() {
    let root = root_with(&[("rust-review", REVIEW)]);
    let found = discover_layer(root.path(), CommandLayer::User);
    let command = &found.commands[0];
    let loaded = read_command(&command.path).expect("the discovered body");
    assert_eq!(loaded.content, command.content);
    assert_eq!(loaded.body, "\nRead the unsafe blocks first.\n");
    let rewritten = "---\ndescription: Edited\n---\nCurrent body.\n";
    fs::write(&command.path, rewritten).expect("rewrite the body");
    let current = read_command(&command.path).expect("the current body");
    assert_eq!(current.body, "Current body.\n");
    assert_eq!(current.description, "Edited");
    assert_eq!(current.content, ContentDigest::of(rewritten.as_bytes()));
    assert_ne!(current.content, command.content);
    fs::write(&command.path, "---\nname: other\n---\nBody.\n").expect("a mismatched name");
    assert!(read_command(&command.path).is_err());
    fs::remove_file(&command.path).expect("remove the command");
    assert!(
        read_command(&command.path)
            .expect_err("a missing file")
            .contains("cannot be read")
    );
}

#[test]
fn expansion_replaces_every_placeholder_once_and_trims_arguments() {
    assert_eq!(
        expand("Review $ARGUMENTS for bugs.", "  src/lib.rs\n"),
        "Review src/lib.rs for bugs."
    );
    assert_eq!(
        expand("$ARGUMENTS/$ARGUMENTS", " literal $ARGUMENTS "),
        "literal $ARGUMENTS/literal $ARGUMENTS"
    );
    assert_eq!(expand("a$ARGUMENTS$ARGUMENTSb\n", " \t"), "ab\n");
}

#[test]
fn expansion_appends_only_non_empty_arguments_without_a_placeholder() {
    assert_eq!(
        expand("Body. \n\t", "  yesterday only  "),
        "Body.\n\nyesterday only"
    );
    assert_eq!(expand("Body. \n\t", " \n\t"), "Body. \n\t");
    assert_eq!(expand("Body.", "$ARGUMENTS"), "Body.\n\n$ARGUMENTS");
}

#[test]
fn shell_and_embedding_syntax_pass_through_as_text() {
    let body = "!cmd\n$(touch /tmp/never) `whoami` ${HOME} @file $1\n";
    assert_eq!(expand(body, ""), body);
    assert_eq!(expand("!cmd $ARGUMENTS", "$(whoami)"), "!cmd $(whoami)");
}
