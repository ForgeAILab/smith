use stable_ready_queue::{ScheduleError, Task, schedule_batches};

fn task(id: &str, dependencies: &[&str]) -> Task {
    Task::new(id, dependencies.iter().copied())
}

#[test]
fn preserves_declaration_order_within_each_parallel_batch() {
    let tasks = vec![
        task("zeta", &[]),
        task("alpha", &[]),
        task("publish", &["compile", "lint"]),
        task("lint", &["zeta"]),
        task("compile", &["alpha"]),
    ];

    assert_eq!(
        schedule_batches(&tasks),
        Ok(vec![
            vec!["zeta".into(), "alpha".into()],
            vec!["lint".into(), "compile".into()],
            vec!["publish".into()],
        ])
    );
    assert_eq!(tasks[0].id, "zeta", "the input must remain untouched");
}

#[test]
fn a_dependency_cannot_share_its_dependents_batch() {
    let tasks = vec![task("a", &[]), task("b", &["a"]), task("c", &["b"])];
    assert_eq!(
        schedule_batches(&tasks),
        Ok(vec![vec!["a".into()], vec!["b".into()], vec!["c".into()]])
    );
}

#[test]
fn rejects_duplicates_before_other_graph_errors() {
    let tasks = vec![task("same", &["missing"]), task("same", &[])];
    assert_eq!(
        schedule_batches(&tasks),
        Err(ScheduleError::DuplicateTask("same".into()))
    );
}

#[test]
fn reports_the_first_unknown_dependency_in_declaration_order() {
    let tasks = vec![
        task("first", &["missing-z", "missing-a"]),
        task("second", &["also-missing"]),
    ];
    assert_eq!(
        schedule_batches(&tasks),
        Err(ScheduleError::UnknownDependency {
            task: "first".into(),
            dependency: "missing-z".into(),
        })
    );
}

#[test]
fn cycle_diagnostic_is_sorted_and_includes_blocked_tasks() {
    let tasks = vec![
        task("downstream", &["cycle-b"]),
        task("cycle-b", &["cycle-a"]),
        task("independent", &[]),
        task("cycle-a", &["cycle-b"]),
    ];
    assert_eq!(
        schedule_batches(&tasks),
        Err(ScheduleError::Cycle(vec![
            "cycle-a".into(),
            "cycle-b".into(),
            "downstream".into(),
        ]))
    );
}

#[test]
fn empty_input_has_no_batches() {
    assert_eq!(schedule_batches(&[]), Ok(Vec::new()));
}
