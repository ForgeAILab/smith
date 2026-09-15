from pathlib import Path

def replace_once(path, old, new):
    p = Path(path)
    s = p.read_text()
    assert s.count(old) == 1, (path, s.count(old), old[:80])
    p.write_text(s.replace(old, new))

replace_once('crates/smith-tui/src/commands.rs',
'''    if command.argument_hint.is_empty() {
        format!("/{}", command.name)''',
'''    // /status is complete by itself; its diagnostic flag is optional.
    if command.argument_hint.is_empty() || command.name == "status" {
        format!("/{}", command.name)''')
replace_once('crates/smith-tui/src/commands.rs',
'''    #[test]
    fn help_and_completion_share_the_complete_registry() {''',
'''    #[test]
    fn status_completion_remains_a_complete_bare_command() {
        let status = COMMANDS.iter().find(|command| command.name == "status").unwrap();
        assert_eq!(completion(status), "/status");
        assert_eq!(parse(&completion(status)).unwrap(), CommandAction::Status);
        assert_eq!(parse("/status --verbose").unwrap(), CommandAction::Diagnostics);
        let model = COMMANDS.iter().find(|command| command.name == "model").unwrap();
        assert_eq!(completion(model), "/model ");
    }

    #[test]
    fn help_and_completion_share_the_complete_registry() {''')
replace_once('crates/smith-cli/src/local_command.rs',
'''    if controller.operation_in_flight {
        return "cache maintenance: running".to_owned();
    }''',
'''    if controller.operation_in_flight {
        return "cache maintenance: running".to_owned();
    }
    if controller.effective_maintenance == CacheMaintenanceMode::Observe {
        return "cache maintenance: observe only (no background requests)".to_owned();
    }''')
p = 'crates/smith-cli/src/main_tests/local_commands.rs'
replace_once(p,
'''        let commands = [
            CommandAction::Status,
            CommandAction::Context,''',
'''        let commands = [
            CommandAction::Status,
            CommandAction::Diagnostics,
            CommandAction::Context,''')
replace_once(p,
'''                ("status", LocalResultState::Info),
                ("context", LocalResultState::Info),''',
'''                ("status", LocalResultState::Info),
                ("diagnostics", LocalResultState::Info),
                ("context", LocalResultState::Info),''')
replace_once(p,
'''        assert!(
            status_content.contains("~98% input left"),
            "{status_content}"
        );
        assert!(
            status_content.contains("profile: dev · posture build · use main · rev"),
            "{status_content}"
        );
        assert!(status_content.contains("source"), "{status_content}");''',
'''        assert!(status_content.contains("profile: dev"), "{status_content}");
        assert!(status_content.contains("/diagnostics"), "{status_content}");
        assert!(!status_content.contains("posture build"), "{status_content}");
        assert!(!status_content.contains("cache: state"), "{status_content}");
        assert!(!status_content.contains("resume capsule:"), "{status_content}");
        let diagnostics_content = app
            .transcript
            .blocks()
            .iter()
            .find_map(|block| match block {
                Block::LocalResult { title, content, .. } if title == "diagnostics" => {
                    Some(content.as_str())
                }
                _ => None,
            })
            .expect("diagnostic output");
        assert!(
            diagnostics_content.contains("~98% input left"),
            "{diagnostics_content}"
        );
        assert!(
            diagnostics_content.contains("profile: dev · posture build · use main · rev"),
            "{diagnostics_content}"
        );
        assert!(diagnostics_content.contains("source"), "{diagnostics_content}");''')
replace_once(p,
'''    fn disabled_maintenance_is_not_rendered_as_an_authority_failure() {
        let controller = smith_runtime::cache_controller::CacheControllerSnapshot::default();''',
'''    fn disabled_maintenance_is_not_rendered_as_an_authority_failure() {
        let controller = smith_runtime::cache_controller::CacheControllerSnapshot {
            requested_maintenance: smith_runtime::cache_lifecycle::CacheMaintenanceMode::Off,
            effective_maintenance: smith_runtime::cache_lifecycle::CacheMaintenanceMode::Off,
            ..Default::default()
        };''')
with Path(p).open('a') as f:
    f.write('''
    #[test]
    fn observe_only_maintenance_names_its_no_spend_behavior() {
        let controller = smith_runtime::cache_controller::CacheControllerSnapshot::default();
        let rendered = crate::local_command::render_cache_controller_summary(&controller);
        assert_eq!(rendered, "cache maintenance: observe only (no background requests)");
        assert!(!rendered.contains("denied"));
    }
''')
