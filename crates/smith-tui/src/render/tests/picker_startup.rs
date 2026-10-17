// Resource-picker startup and narrow-layout regressions.

use crate::{App, Overlay, ResourceEntry, ResourcePicker, ResourceTarget, Theme};

#[test]
fn narrow_resource_picker_hint_keeps_choose_and_cancel_controls() {
    let mut app = App::new("gpt-5.3", "~/work/api");
    app.open_overlay(Overlay::ResourcePicker {
        picker: ResourcePicker::new(
            "Choose model",
            vec![ResourceEntry::new(
                "local/model",
                "local/model",
                "configured",
            )],
            "run setup",
        ),
        target: ResourceTarget::Model,
        restore_on_escape: "/model".to_owned(),
    });

    let screen = render(&app, 44, 16, Theme::new().without_color().without_motion());
    let hint = screen
        .lines()
        .find(|line| line.contains("enter confirm"))
        .expect("the narrow resource picker hint is rendered");
    assert!(hint.contains("esc cancel"), "{screen}");
    assert!(
        hint.width() <= 44,
        "narrow picker hint overflowed:\n{screen}"
    );
}

#[test]
fn bare_context_draws_the_window_picker_above_the_composer() {
    let mut app = App::new("window-model", "~/work/api");
    app.set_resources(crate::app::RuntimeResources {
        context_windows: ["128k", "1m", "32k"]
            .into_iter()
            .map(|name| {
                ResourceEntry::new(name, name, "select this model context window")
                    .active(name == "128k")
            })
            .collect(),
        context_window: Some("128k".to_owned()),
        ..crate::app::RuntimeResources::default()
    });
    for character in "/context".chars() {
        app.on_key(KeyEvent::new(KeyCode::Char(character), KeyModifiers::NONE));
    }
    app.on_key(KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE));

    let screen = render(&app, 100, 24, Theme::new().without_color().without_motion());
    let rows = screen.lines().collect::<Vec<_>>();
    let title = rows
        .iter()
        .position(|row| row.starts_with("  Choose context window · type to filter"))
        .unwrap_or_else(|| panic!("the window picker is drawn:\n{screen}"));
    assert!(
        rows[title + 1].starts_with("❯ 128k") && rows[title + 1].ends_with("✓ current"),
        "{screen}"
    );
    let composer = rows
        .iter()
        .position(|row| row.starts_with("> "))
        .unwrap_or_else(|| panic!("the composer is drawn:\n{screen}"));
    assert!(title < composer, "{screen}");
    assert!(
        rows[title..composer]
            .iter()
            .any(|row| row.starts_with("  1m")),
        "{screen}"
    );
}
