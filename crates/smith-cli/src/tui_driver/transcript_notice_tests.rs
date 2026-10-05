use super::change_notice;
use smith_tools::{EditMutation, ToolMutation, TurnChangeSet};

fn set(mutations: Vec<ToolMutation>) -> TurnChangeSet {
    TurnChangeSet {
        turn: 1,
        mutations,
        undone: false,
    }
}

fn edit(before: &[u8], after: &[u8]) -> ToolMutation {
    ToolMutation::Exact(EditMutation {
        call_id: "edit".to_owned(),
        path: "/repo/src/retry.rs".into(),
        before: Some(before.to_vec()),
        after: Some(after.to_vec()),
        before_hash: String::new(),
        after_hash: String::new(),
        recovery_path: None,
    })
}

#[test]
fn empty_and_ambiguous_only_turns_do_not_claim_changed_files() {
    assert_eq!(change_notice(&set(Vec::new())), None);
    assert_eq!(
        change_notice(&set(vec![ToolMutation::Ambiguous {
            call_id: "ls".to_owned(),
            tool: "shell".to_owned()
        }])),
        None
    );
    assert_eq!(change_notice(&set(vec![edit(b"same", b"same")])), None);
}

#[test]
fn a_confirmed_file_change_keeps_its_recovery_notice() {
    let changed = set(vec![edit(b"before", b"after")]);
    assert_eq!(
        change_notice(&changed).as_deref(),
        Some("Smith turn 1 · undo available")
    );
    let mixed = set(vec![
        edit(b"before", b"after"),
        ToolMutation::Ambiguous {
            call_id: "shell".to_owned(),
            tool: "shell".to_owned(),
        },
    ]);
    assert!(
        change_notice(&mixed)
            .unwrap()
            .contains("contains ambiguous changes")
    );
    let mut undone = changed;
    undone.undone = true;
    assert_eq!(change_notice(&undone), None);
}
