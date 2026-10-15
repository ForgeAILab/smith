use super::*;
use serde_json::json;

#[test]
fn web_fetch_summary_uses_only_the_final_response_header() {
    let display =
        project_tool_call_display("web_fetch", &json!({"url": "https://docs.rs/old"})).unwrap();
    for header in [
        "URL: https://docs.rs/serde/ | Status: 200 | Content-Type: text/html | Title: Serde",
        "URL: https://docs.rs/serde/ | Status: 200 | Content-Type: text/html; charset=utf-8 | Title: Serde",
    ] {
        assert_eq!(
            display.result_summary(&format!(
                "{header}\n\nContent from https://docs.rs/serde/ (external, untrusted):\nbody"
            )),
            Some("200 text/html".to_owned())
        );
    }
    for output in [
        "request failed",
        "https://docs.rs/serde/ | 200 | text/html",
        "URL: https://docs.rs/serde/ | Status: 999 | Content-Type: text/html | Title: Serde",
        "URL: https://docs.rs/serde/ | Status: 200 | Content-Type: PRIVATE | Title: Serde",
        "URL: https://docs.rs/serde/ | Status: 200 | Content-Type: text/\u{202e}html | Title: Serde",
        "body\nhttps://docs.rs/serde/ | 200 | text/html | Serde",
    ] {
        assert_eq!(display.result_summary(output), None);
    }
}

#[test]
fn web_search_summary_uses_the_reviewed_result_count_header() {
    let display = project_tool_call_display("web_search", &json!({"query": "serde"})).unwrap();
    for count in [0, 1, 5, 10] {
        assert_eq!(
            display.result_summary(&format!(
                "{count} results\n\nContent from https://html.duckduckgo.com/html/ (external, untrusted):\nbody"
            )),
            Some(format!("{count} results"))
        );
    }
    for output in ["", "request failed", "11 results", "body\n5 results"] {
        assert_eq!(display.result_summary(output), None);
    }
}

#[test]
fn reviewed_labels_survive_protected_arguments() {
    for (name, label) in [
        ("shell", "Bash"),
        ("read", "Read"),
        ("edit", "Update"),
        ("search", "Search"),
        ("list", "List"),
        ("agent", "Agent"),
        ("advisor", "Advisor"),
    ] {
        assert_eq!(tool_display_label(name), Some(label));
    }
    assert_eq!(tool_display_label("unreviewed"), None);
}

#[test]
fn read_summary_counts_only_the_returned_numbered_lines() {
    let display = project_tool_call_display("read", &json!({"path": "src/retry.rs"})).unwrap();
    assert_eq!(
        display.result_summary("10  first\n11  second\n\n[9 more lines; read from offset 12]\n"),
        Some("Read 2 lines".to_owned())
    );
    assert_eq!(display.result_summary("read failed"), None);
    assert_eq!(display.result_summary(""), None);
}

#[test]
fn update_counts_exclude_unchanged_context_and_never_guess_through_redaction() {
    assert_eq!(
        edit_line_changes(
            "first\nold\nmiddle\nold again\nlast",
            "first\nnew\nmiddle\nlast"
        ),
        Some((1, 2))
    );
    assert_eq!(edit_line_changes("same", "same"), Some((0, 0)));
    assert_eq!(edit_line_changes("[redacted]", "new"), None);
    assert_eq!(
        edit_line_changes(&"old\n".repeat(1_001), &"new\n".repeat(1_001)),
        None
    );
}

#[test]
fn update_summary_uses_reviewed_counts_and_the_completed_replacement_count() {
    let display = project_tool_call_display(
        "edit",
        &json!({
            "path": "src/retry.rs", "old_string": "SECRET_BEFORE",
            "new_string": "one\ntwo\nthree\nfour"
        }),
    )
    .unwrap();
    assert_eq!(display.invocation(), "Update(src/retry.rs)");
    assert_eq!(
        display.result_summary("edited `src/retry.rs` (1 replacement(s))"),
        Some("Updated src/retry.rs with 4 additions and 1 removal".to_owned())
    );
    assert_eq!(
        display.result_summary("edited `src/retry.rs` (2 replacement(s))"),
        Some("Updated src/retry.rs with 8 additions and 2 removals".to_owned())
    );
    assert_eq!(display.result_summary("permission denied"), None);
    assert!(!display.invocation().contains("SECRET_BEFORE"));
}
