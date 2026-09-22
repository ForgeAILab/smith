// Resource-picker startup and narrow-layout regressions.

use crate::{App, Overlay, ResourceEntry, ResourcePicker, ResourceTarget, Theme};

#[test]
fn narrow_resource_picker_hint_keeps_choose_and_cancel_controls() {
    let mut app = App::new("gpt-5.3", "~/work/api");
    app.overlay = Some(Overlay::ResourcePicker {
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
        .find(|line| line.contains("enter choose"))
        .expect("the narrow resource picker hint is rendered");
    assert!(hint.contains("esc cancel"), "{screen}");
    assert!(
        hint.width() <= 44,
        "narrow picker hint overflowed:\n{screen}"
    );
}
