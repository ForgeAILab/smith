from pathlib import Path
import subprocess
p = Path('crates/smith-tui/src/app/resources.rs')
s = p.read_text()
old = '                CommandAction::Status => "status",\n'
assert s.count(old) == 1
s = s.replace(old, old + '                CommandAction::Diagnostics => "diagnostics",\n')
p.write_text(s)

# This is an include! fragment indented for its enclosing test module;
# formatting it as an independent crate needlessly rewrites every old test.
p = Path('crates/smith-cli/src/main_tests/local_commands.rs')
base = subprocess.check_output(['git', 'show', '555d1cb4c95432db95fc626258c8a57908f7c82b:' + str(p)], text=True)
p.write_text(base.rstrip() + '''

    #[test]
    fn disabled_maintenance_is_not_rendered_as_an_authority_failure() {
        let controller = smith_runtime::cache_controller::CacheControllerSnapshot::default();
        let rendered = crate::local_command::render_cache_controller_summary(&controller);
        assert_eq!(rendered, "cache maintenance: off");
        assert!(!rendered.contains("denied"));
        assert!(!rendered.contains("lease"));
        assert!(!rendered.contains("idle attempted"));
    }
''')
