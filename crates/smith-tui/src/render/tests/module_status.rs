use super::*;
use smith_client::status::{ModuleStatusItem, ModuleStatusSeverity};

fn item(name: &str, label: &str, severity: Option<ModuleStatusSeverity>) -> ModuleStatusItem {
    ModuleStatusItem {
        name: name.to_owned(),
        label: label.to_owned(),
        severity,
    }
}

fn app_with_status() -> (App, String) {
    let mut app = App::new("gpt-5.3", "~/work/api");
    app.status.approval_mode = Some("ask".to_owned());
    app.status.context_window = Some("872k".to_owned());
    let built_in = format!(
        "gpt-5.3 · {} · ask · 872k · ~/work/api · unknown ctx",
        app.status.agent
    );
    (app, built_in)
}

#[test]
fn module_status_item_is_visible_without_color() {
    let (mut app, built_in) = app_with_status();
    app.set_module_status(vec![item(
        "budget-pressure",
        "context pressure warning",
        Some(ModuleStatusSeverity::Warning),
    )]);

    let screen = render(&app, 120, 16, Theme::new().without_color());
    let footer = screen.lines().last().unwrap_or_default();
    insta_like(
        footer,
        &["? for shortcuts", &built_in, "context pressure warning"],
    );
    assert!(!screen.contains("budget-pressure"), "{screen}");
}

#[test]
fn oversized_module_status_is_truncated_after_the_complete_built_in_status() {
    let (mut app, built_in) = app_with_status();
    let label = format!("warning: {}", "context pressure ".repeat(100));
    app.set_module_status(vec![item("pressure", &label, None)]);

    let screen = render(&app, 100, 16, Theme::new().without_color());
    let footer = screen.lines().last().unwrap_or_default();
    insta_like(footer, &["? for shortcuts", &built_in, "warning: ", "…"]);
    assert!(!footer.contains(&label), "{footer}");
    assert!(footer.width() <= 100, "{footer}");
}

#[test]
fn module_status_update_removes_items_absent_from_the_new_list() {
    let (mut app, built_in) = app_with_status();
    app.set_module_status(vec![
        item("pressure", "pressure warning", None),
        item("healthy", "module healthy", None),
    ]);
    let theme = Theme::new().without_color();
    insta_like(
        &render(&app, 140, 16, theme),
        &["pressure warning", "module healthy"],
    );

    app.set_module_status(vec![item("healthy", "module healthy", None)]);
    let screen = render(&app, 140, 16, theme);
    insta_like(&screen, &[&built_in, "module healthy"]);
    assert!(!screen.contains("pressure warning"), "{screen}");

    app.set_module_status(Vec::new());
    let screen = render(&app, 140, 16, theme);
    assert!(!screen.contains("module healthy"), "{screen}");
    assert!(screen.contains(&built_in), "{screen}");
}

#[test]
fn narrow_terminal_drops_module_status_before_any_built_in_status() {
    let (mut app, _) = app_with_status();
    let theme = Theme::new().without_color();
    for width in [40, 44, 60] {
        app.set_module_status(Vec::new());
        let baseline = render(&app, width, 16, theme);
        app.set_module_status(vec![item("pressure", "W", None)]);
        assert_eq!(render(&app, width, 16, theme), baseline, "width {width}");
    }
}

#[test]
fn module_status_is_dropped_when_only_the_separator_would_fit() {
    let (mut app, built_in) = app_with_status();
    let theme = Theme::new().without_color();
    let minimum = built_in.width() + "? for shortcuts".width() + 4;
    for spare in 0..=3 {
        let width = u16::try_from(minimum + spare).expect("a terminal width");
        app.set_module_status(Vec::new());
        let baseline = render(&app, width, 16, theme);
        assert!(baseline.contains(&built_in), "{baseline}");
        app.set_module_status(vec![item("pressure", "W", None)]);
        assert_eq!(render(&app, width, 16, theme), baseline, "spare {spare}");
    }
}

#[test]
fn module_status_preserves_the_order_supplied_by_the_host() {
    let (mut app, built_in) = app_with_status();
    let theme = Theme::new().without_color();
    app.set_module_status(vec![
        item("z-last", "first label", None),
        item("a-first", "second label", None),
    ]);
    let screen = render(&app, 140, 16, theme);
    assert!(
        screen.contains(&format!("{built_in} · first label · second label")),
        "{screen}"
    );
    assert_eq!(render(&app, 140, 16, theme), screen);

    app.set_module_status(vec![
        item("a-first", "second label", None),
        item("z-last", "first label", None),
    ]);
    let screen = render(&app, 140, 16, theme);
    assert!(
        screen.contains(&format!("{built_in} · second label · first label")),
        "{screen}"
    );
}

#[test]
fn module_status_uses_existing_severity_tones() {
    for (severity, tone) in [
        (None, Tone::Default),
        (Some(ModuleStatusSeverity::Info), Tone::Default),
        (Some(ModuleStatusSeverity::Warning), Tone::Warning),
        (Some(ModuleStatusSeverity::Error), Tone::Danger),
    ] {
        let (mut app, _) = app_with_status();
        app.set_module_status(vec![item("pressure", "module warning", severity)]);
        let mut terminal = Terminal::new(TestBackend::new(120, 16)).expect("a terminal");
        let theme = Theme::new();
        terminal
            .draw(|frame| draw(frame, &app, theme))
            .expect("a frame");
        let buffer = terminal.backend().buffer();
        let row = buffer.area.height - 1;
        let column = (0..buffer.area.width)
            .find(|&column| buffer[(column, row)].symbol() == "m")
            .expect("module label");
        assert_eq!(
            buffer[(column, row)].fg,
            theme.style(tone).fg.unwrap_or(Color::Reset)
        );
    }
}

#[test]
fn module_status_clips_unicode_labels_to_cells_and_keeps_them_on_one_row() {
    let (mut app, built_in) = app_with_status();
    app.set_module_status(vec![item(
        "pressure",
        &format!("warning:\n{}", "警".repeat(100)),
        None,
    )]);
    let screen = render(&app, 100, 16, Theme::new().without_color());
    let footer = screen.lines().last().unwrap_or_default();
    insta_like(footer, &[&built_in, "warning: 警", "…"]);
    assert!(footer.width() <= 100, "{footer}");
    assert_eq!(
        screen
            .lines()
            .filter(|line| line.contains("warning:"))
            .count(),
        1
    );
}
