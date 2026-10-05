use super::recovery::{textual_forward, textual_patch, textual_reverse};
use super::*;

fn exact(path: &Path, before: Option<&[u8]>, after: &[u8]) -> ToolMutation {
    ToolMutation::Exact(EditMutation {
        call_id: "call-1".to_owned(),
        path: path.to_path_buf(),
        before: before.map(<[u8]>::to_vec),
        after: Some(after.to_vec()),
        before_hash: hash(before),
        after_hash: hash(Some(after)),
        recovery_path: None,
    })
}

#[test]
fn exact_turn_previews_and_undoes_after_postimage_validation() {
    let dir = tempfile::tempdir().expect("temp");
    let path = dir.path().join("file.txt");
    std::fs::write(&path, b"after\n").expect("after");
    let recorder = ChangeRecorder::new(None);
    recorder.start_turn();
    recorder.record(exact(&path, Some(b"before\n"), b"after\n"));
    let set = recorder.finish_turn().expect("set");
    assert!(set.is_fully_attributable());
    assert!(recorder.undo_preview().expect("preview").contains("-after"));
    recorder.undo_latest().expect("undo");
    assert_eq!(std::fs::read(&path).expect("read"), b"before\n");
    assert!(recorder.undo_latest().is_err(), "repeated undo must refuse");
}

#[test]
fn recovery_patches_show_a_four_line_addition_with_three_context_lines() {
    let before = (1..=20).map(|n| format!("line {n}\n")).collect::<String>();
    let after = before.replace("line 10\n", "line 10\nnew a\nnew b\nnew c\nnew d\n");
    let mutation = exact(
        Path::new("file.txt"),
        Some(before.as_bytes()),
        after.as_bytes(),
    );
    let ToolMutation::Exact(edit) = mutation else {
        unreachable!()
    };

    let forward = textual_forward(&edit);
    assert_eq!(
        forward,
        "@@ -8,6 +8,10 @@\n line 8\n line 9\n line 10\n+new a\n+new b\n+new c\n+new d\n line 11\n line 12\n line 13\n"
    );
    assert_eq!(
        textual_reverse(&edit),
        "@@ -8,10 +8,6 @@\n line 8\n line 9\n line 10\n-new a\n-new b\n-new c\n-new d\n line 11\n line 12\n line 13\n"
    );
}

#[test]
fn recovery_patches_split_distant_edits_and_merge_adjacent_context() {
    let before = (1..=30).map(|n| format!("line {n}\n")).collect::<String>();
    let after = before
        .replace("line 5\n", "five\n")
        .replace("line 25\n", "twenty five\n");
    let patch = textual_patch(Some(before.as_bytes()), Some(after.as_bytes()));
    assert_eq!(
        patch
            .lines()
            .filter(|line| line.starts_with("@@ "))
            .collect::<Vec<_>>(),
        ["@@ -2,7 +2,7 @@", "@@ -22,7 +22,7 @@"]
    );
    assert!(patch.contains("-line 5\n+five\n"), "{patch}");
    assert!(patch.contains("-line 25\n+twenty five\n"), "{patch}");
    assert!(!patch.contains(" line 15\n"), "{patch}");

    let nearby = before
        .replace("line 5\n", "five\n")
        .replace("line 10\n", "ten\n");
    let patch = textual_patch(Some(before.as_bytes()), Some(nearby.as_bytes()));
    assert_eq!(
        patch.lines().filter(|line| line.starts_with("@@ ")).count(),
        1
    );

    let shifted = before
        .replace("line 5\n", "line 5\nextra\n")
        .replace("line 25\n", "");
    let patch = textual_patch(Some(before.as_bytes()), Some(shifted.as_bytes()));
    assert_eq!(
        patch
            .lines()
            .filter(|line| line.starts_with("@@ "))
            .collect::<Vec<_>>(),
        ["@@ -3,6 +3,7 @@", "@@ -22,7 +23,6 @@"]
    );
}

#[test]
fn recovery_patches_show_created_and_deleted_files_as_one_complete_hunk() {
    let text = (1..=20).map(|n| format!("line {n}\n")).collect::<String>();
    let created = textual_patch(None, Some(text.as_bytes()));
    let deleted = textual_patch(Some(text.as_bytes()), None);
    assert_eq!(created.lines().next(), Some("@@ -0,0 +1,20 @@"));
    assert_eq!(deleted.lines().next(), Some("@@ -1,20 +0,0 @@"));
    assert_eq!(
        created.lines().filter(|line| line.starts_with('+')).count(),
        20
    );
    assert_eq!(
        deleted.lines().filter(|line| line.starts_with('-')).count(),
        20
    );
    assert_eq!(textual_patch(None, Some(b"")), "@@ -0,0 +0,0 @@\n");
}

#[test]
fn recovery_patches_omit_single_line_counts_and_keep_empty_ranges() {
    for (old, new, expected) in [
        (Some(&b"old\n"[..]), Some(&b"new\n"[..]), "@@ -1 +1 @@"),
        (
            Some(&b"old\n"[..]),
            Some(&b"new\nextra\n"[..]),
            "@@ -1 +1,2 @@",
        ),
        (
            Some(&b"old\nextra\n"[..]),
            Some(&b"new\n"[..]),
            "@@ -1,2 +1 @@",
        ),
        (None, Some(&b"new\n"[..]), "@@ -0,0 +1 @@"),
        (Some(&b"old\n"[..]), None, "@@ -1 +0,0 @@"),
        (Some(&b""[..]), Some(&b"new\n"[..]), "@@ -0,0 +1 @@"),
        (Some(&b"old\n"[..]), Some(&b""[..]), "@@ -1 +0,0 @@"),
    ] {
        let patch = textual_patch(old, new);
        assert_eq!(patch.lines().next(), Some(expected), "{patch}");
    }
}

#[test]
fn recovery_patches_keep_repeated_lines_and_eof_changes_honest() {
    assert_eq!(
        textual_patch(Some(b"a\nx\nx\nz\n"), Some(b"a\nx\ny\nx\nz\n")),
        "@@ -1,4 +1,5 @@\n a\n x\n+y\n x\n z\n"
    );
    assert_eq!(
        textual_patch(Some(b"a\n"), Some(b"a")),
        "@@ -1 +1 @@\n-a\n+a\n\\ No newline at end of file\n"
    );
    assert!(textual_patch(Some(b"same\n"), Some(b"same\n")).is_empty());
}

#[test]
fn recovery_preview_paths_are_relative_inside_the_project_and_absolute_outside() {
    let dir = tempfile::tempdir().expect("temp");
    let root = dir.path().canonicalize().expect("root");
    let path = root.join("src/lib.rs");
    let outside = root.with_file_name("other-project").join("src/lib.rs");
    let recorder = ChangeRecorder::new(None).with_project_root(&root);
    recorder.start_turn();
    recorder.record(exact(&path, Some(b"before\n"), b"after\n"));
    recorder.record(exact(&outside, Some(b"old\n"), b"new\n"));
    let set = recorder.finish_turn().expect("turn");
    let undo = recorder.undo_preview().expect("undo preview");
    let redo = redo_preview_text(&set, RedoDirection::ReapplyUndoneTurn, Some(&root));
    for preview in [&undo, &redo] {
        assert!(preview.contains("--- current src/lib.rs\n"), "{preview}");
        assert!(!preview.contains(&path.display().to_string()), "{preview}");
        assert!(
            preview.contains(&outside.display().to_string()),
            "{preview}"
        );
    }
    assert!(undo.contains("+++ restore src/lib.rs\n"));
    assert!(redo.contains("+++ reapply src/lib.rs\n"));
}

#[test]
fn recovery_preview_cancel_and_apply_journal_the_same_text_fingerprint() {
    let dir = tempfile::tempdir().expect("temp");
    let root = dir.path().canonicalize().expect("root");
    let path = root.join("file.txt");
    let journal = root.join("changes.jsonl");
    std::fs::write(&path, b"after\n").expect("write");
    let recorder = ChangeRecorder::new(Some(journal.clone())).with_project_root(&root);
    recorder.start_turn();
    recorder.record(exact(&path, Some(b"before\n"), b"after\n"));
    recorder.finish_turn().expect("turn");
    let undo = recorder.undo_preview().expect("undo preview");
    recorder.record_undo_cancelled();
    recorder.undo_latest().expect("undo");
    let redo = recorder.redo_preview().expect("redo preview");
    recorder.record_redo_cancelled();
    recorder.redo_latest().expect("redo");

    let stored = std::fs::read_to_string(journal).expect("journal");
    let events = stored
        .lines()
        .map(|line| serde_json::from_str::<Value>(line).expect("record"))
        .collect::<Vec<_>>();
    for (operation, preview) in [("undo", undo), ("redo", redo)] {
        assert!(preview.contains("@@ -1 +1 @@"), "{preview}");
        let fingerprint = hash(Some(preview.as_bytes()));
        let requests = events
            .iter()
            .filter(|event| event["fingerprint"].is_string() && event["operation"] == operation)
            .collect::<Vec<_>>();
        assert_eq!(requests.len(), 3, "{requests:?}");
        for (request, outcome) in requests.iter().zip(["previewed", "cancelled", "applied"]) {
            assert_eq!(request["fingerprint"], fingerprint);
            assert_eq!(request["outcome"], outcome);
        }
    }
    assert_eq!(std::fs::read(path).expect("read"), b"after\n");
    assert!(!stored.contains("before\\n") && !stored.contains("after\\n"));
}

#[test]
fn concurrent_edit_refuses_without_touching_any_path() {
    let dir = tempfile::tempdir().expect("temp");
    let first = dir.path().join("first.txt");
    let second = dir.path().join("second.txt");
    std::fs::write(&first, b"after one\n").expect("first");
    std::fs::write(&second, b"after two\n").expect("second");
    let recorder = ChangeRecorder::new(None);
    recorder.start_turn();
    recorder.record(exact(&first, Some(b"before one\n"), b"after one\n"));
    recorder.record(exact(&second, Some(b"before two\n"), b"after two\n"));
    recorder.finish_turn();

    std::fs::write(&second, b"user edit\n").expect("edit");
    assert!(recorder.undo_latest().is_err());
    assert_eq!(std::fs::read(&first).expect("first"), b"after one\n");
    assert_eq!(std::fs::read(&second).expect("second"), b"user edit\n");
}

#[test]
fn a_turn_with_nothing_but_ambiguous_deltas_has_no_undo_candidate() {
    let recorder = ChangeRecorder::new(None);
    recorder.start_turn();
    recorder.record(ToolMutation::Ambiguous {
        call_id: "shell-1".to_owned(),
        tool: "shell".to_owned(),
    });
    let set = recorder.finish_turn().expect("set");
    assert!(!set.is_fully_attributable());
    assert!(!set.has_exact_mutations());
    assert!(
        recorder
            .undo_preview()
            .unwrap_err()
            .message
            .contains("attribute file by file")
    );
    assert!(recorder.undo_latest().is_err());
}

#[test]
fn a_mixed_turn_undoes_smiths_own_edits_and_names_what_it_leaves() {
    let dir = tempfile::tempdir().expect("temp");
    let edited = dir.path().join("edited.txt");
    let shelled = dir.path().join("shelled.txt");
    std::fs::write(&edited, b"after\n").expect("edited");
    std::fs::write(&shelled, b"shell wrote this\n").expect("shelled");
    let recorder = ChangeRecorder::new(None);
    recorder.start_turn();
    recorder.record(exact(&edited, Some(b"before\n"), b"after\n"));
    recorder.record(ToolMutation::Ambiguous {
        call_id: "shell-1".to_owned(),
        tool: "shell".to_owned(),
    });
    let set = recorder.finish_turn().expect("set");
    assert!(!set.is_fully_attributable());
    assert!(set.has_exact_mutations());

    let preview = recorder.undo_preview().expect("preview");
    assert!(preview.contains("-after"), "{preview}");
    assert!(
        preview.contains("shell") && preview.contains("left untouched"),
        "the preview must name the delta it will not reverse: {preview}"
    );

    recorder.undo_latest().expect("undo");
    assert_eq!(std::fs::read(&edited).expect("edited"), b"before\n");
    assert_eq!(
        std::fs::read(&shelled).expect("shelled"),
        b"shell wrote this\n",
        "an unattributable path must survive the partial undo untouched"
    );

    // The exact half is recoverable in both directions.
    recorder.redo_latest().expect("redo");
    assert_eq!(std::fs::read(&edited).expect("edited"), b"after\n");
}

#[test]
fn a_mixed_turn_refuses_when_its_own_edit_was_overwritten() {
    let dir = tempfile::tempdir().expect("temp");
    let edited = dir.path().join("edited.txt");
    std::fs::write(&edited, b"after\n").expect("edited");
    let recorder = ChangeRecorder::new(None);
    recorder.start_turn();
    recorder.record(exact(&edited, Some(b"before\n"), b"after\n"));
    recorder.record(ToolMutation::Ambiguous {
        call_id: "shell-1".to_owned(),
        tool: "shell".to_owned(),
    });
    recorder.finish_turn().expect("set");

    // A formatter run after the edit is exactly the case the post-image
    // check exists for: the recorded reverse no longer describes the file.
    std::fs::write(&edited, b"after, reformatted\n").expect("overwrite");
    assert!(recorder.undo_latest().is_err());
    assert_eq!(
        std::fs::read(&edited).expect("edited"),
        b"after, reformatted\n"
    );
}

#[test]
fn persisted_metadata_contains_hashes_but_not_file_contents() {
    let dir = tempfile::tempdir().expect("temp");
    let path = dir.path().join("file.txt");
    let journal = dir.path().join("changes.jsonl");
    let recorder = ChangeRecorder::new(Some(journal.clone()));
    recorder.start_turn();
    recorder.record(exact(
        &path,
        Some(b"secret-before-value"),
        b"secret-after-value",
    ));
    recorder.finish_turn();
    let stored = std::fs::read_to_string(journal).expect("journal");
    assert!(stored.contains("before_hash"));
    assert!(!stored.contains("secret-before-value"));
    assert!(!stored.contains("secret-after-value"));
}

#[test]
fn cancelled_undo_is_audited_without_file_content() {
    let dir = tempfile::tempdir().expect("temp");
    let path = dir.path().join("file.txt");
    let journal = dir.path().join("changes.jsonl");
    let recorder = ChangeRecorder::new(Some(journal.clone()));
    recorder.start_turn();
    recorder.record(exact(&path, Some(b"before"), b"after"));
    recorder.finish_turn();
    recorder.undo_preview().expect("preview");
    recorder.record_undo_cancelled();

    let stored = std::fs::read_to_string(journal).expect("journal");
    assert!(stored.contains(r#""outcome":"cancelled""#));
    assert!(!stored.contains(r#""before""#));
    assert!(!stored.contains(r#""after""#));
}

#[test]
fn repeated_edits_to_one_path_restore_the_original_image() {
    let dir = tempfile::tempdir().expect("temp");
    let path = dir.path().join("file.txt");
    std::fs::write(&path, b"third\n").expect("write");
    let recorder = ChangeRecorder::new(None);
    recorder.start_turn();
    recorder.record(exact(&path, Some(b"first\n"), b"second\n"));
    recorder.record(exact(&path, Some(b"second\n"), b"third\n"));
    let set = recorder.finish_turn().expect("set");
    assert_eq!(set.mutations.len(), 1);
    recorder.undo_latest().expect("undo");
    assert_eq!(std::fs::read(path).expect("read"), b"first\n");
}

#[test]
fn exact_undo_can_be_previewed_and_redone_once() {
    let dir = tempfile::tempdir().expect("temp");
    let path = dir.path().join("file.txt");
    std::fs::write(&path, b"after\n").expect("write");
    let recorder = ChangeRecorder::new(None);
    recorder.start_turn();
    recorder.record(exact(&path, Some(b"before\n"), b"after\n"));
    recorder.finish_turn().expect("set");
    recorder.undo_latest().expect("undo");

    let preview = recorder.redo_preview().expect("redo preview");
    assert!(preview.contains("-before"));
    assert!(preview.contains("+after"));
    recorder.redo_latest().expect("redo");

    assert_eq!(std::fs::read(&path).expect("read"), b"after\n");
    assert!(recorder.redo_latest().is_err(), "redo is single-use");
}

#[test]
fn concurrent_edit_refuses_redo_without_touching_any_path() {
    let dir = tempfile::tempdir().expect("temp");
    let first = dir.path().join("first.txt");
    let second = dir.path().join("second.txt");
    std::fs::write(&first, b"after one\n").expect("first");
    std::fs::write(&second, b"after two\n").expect("second");
    let recorder = ChangeRecorder::new(None);
    recorder.start_turn();
    recorder.record(exact(&first, Some(b"before one\n"), b"after one\n"));
    recorder.record(exact(&second, Some(b"before two\n"), b"after two\n"));
    recorder.finish_turn().expect("set");
    recorder.undo_latest().expect("undo");

    std::fs::write(&second, b"user edit\n").expect("edit");
    let error = recorder.redo_latest().expect_err("conflict");
    assert!(error.message.contains("/diff"));
    assert!(error.message.contains("/timeline"));
    assert_eq!(std::fs::read(&first).expect("first"), b"before one\n");
    assert_eq!(std::fs::read(&second).expect("second"), b"user edit\n");
}

#[test]
fn exact_selective_revert_is_a_redo_candidate() {
    let dir = tempfile::tempdir().expect("temp");
    let path = dir.path().join("file.txt");
    std::fs::write(&path, b"restored\n").expect("reverted state");
    let recorder = ChangeRecorder::new(None);
    recorder.record_recovery(
        path.clone(),
        Some(b"changed\n".to_vec()),
        Some(b"restored\n".to_vec()),
        "revert",
        None,
    );

    let preview = recorder.redo_preview().expect("redo preview");
    assert!(preview.contains("-restored"));
    assert!(preview.contains("+changed"));
    recorder.redo_latest().expect("redo revert");

    assert_eq!(std::fs::read(path).expect("read"), b"changed\n");
    assert!(recorder.redo_preview().is_err(), "redo is single-use");
}

#[test]
fn ambiguous_shell_delta_is_never_redoable() {
    let recorder = ChangeRecorder::new(None);
    recorder.start_turn();
    recorder.record(ToolMutation::Ambiguous {
        call_id: "shell-1".to_owned(),
        tool: "shell".to_owned(),
    });
    recorder.finish_turn().expect("set");
    assert!(recorder.redo_preview().is_err());
    assert!(recorder.redo_latest().is_err());
}

#[test]
fn resumed_metadata_stays_visible_but_is_not_synthesized_into_undo_images() {
    let dir = tempfile::tempdir().expect("temp");
    let journal = dir.path().join("changes.jsonl");
    std::fs::write(
            &journal,
            r#"{"record":"turn_completed","schema_version":1,"turn":7,"fully_attributable":true,"mutations":[]}
"#,
        )
        .expect("journal");
    let recorder = ChangeRecorder::new(Some(journal));
    assert!(recorder.has_historical_records());
    assert_eq!(
        recorder.undo_preview().unwrap_err().message,
        "undo is not available for turns from before this session was resumed"
    );

    recorder.start_turn();
    recorder.record(ToolMutation::Ambiguous {
        call_id: "shell".to_owned(),
        tool: "shell".to_owned(),
    });
    assert_eq!(recorder.finish_turn().expect("set").turn, 8);
}
