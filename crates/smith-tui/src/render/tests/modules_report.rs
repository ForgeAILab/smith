use smith_client::modules_report::{Layer, ModuleBlockReason, ModuleOrigin, ModuleRow, ModuleState, ModulesReport, Source};

use super::{App, LocalResult, Theme, transcript_lines};

#[test]
fn every_module_state_renders_locally_with_its_reason_and_origin() {
    for (state, expected) in [
        (ModuleState::Mounted, "mounted"),
        (ModuleState::Off, "off"),
        (ModuleState::NotBuilt, "not built"),
        (ModuleState::Blocked { reason: ModuleBlockReason::Requirement("dependency".into()) }, "requires dependency"),
        (ModuleState::Blocked { reason: ModuleBlockReason::Cycle(vec!["a".into(), "b".into()]) }, "requirement cycle: a, b"),
        (ModuleState::Failed { reason: "fixture failure".into() }, "failed · fixture failure"),
        (ModuleState::Inactive { reason: "no image binding".into() }, "inactive · no image binding"),
    ] {
        let report = ModulesReport { rows: vec![ModuleRow {
            id: "external".into(), description: "External image tool".into(), state,
            source: Source::file(Layer::UserFile, "/user/.smith/config.toml", "tools.image_generation.enabled"),
            origin: ModuleOrigin::ThirdParty { crate_name: "external-crate".into() },
        }] };
        let result = LocalResult::Modules(Box::new(report));
        assert_eq!(result.title(), "modules");
        assert_eq!(result.state(), smith_client::local_result::LocalResultState::Info);
        let mut app = App::new("model", "project");
        app.show_local_report(result);
        assert!(app.overlay.is_none());
        for width in [44, 100, 180] {
            let text = transcript_lines(&app, Theme::new().without_color(), width).iter()
                .map(ToString::to_string).collect::<Vec<_>>().join("\n");
            // At narrow widths the renderer wraps words rather than hiding reasons.
            let text = text.split_whitespace().collect::<Vec<_>>().join(" ");
            for label in ["/modules", expected, "External image tool", "third-party", "external-crate", "user config"] {
                assert!(text.contains(label), "{width}: {text}");
            }
            if width >= 100 { assert!(text.contains("tools.image_generation.enabled"), "{text}"); }
        }
    }
}
