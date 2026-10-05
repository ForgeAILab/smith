use super::*;

#[test]
fn non_empty_legacy_auto_approval_fails_during_resolution_with_migration_guidance() {
    let fixture = Fixture::new();
    fixture.write_project(&format!(
        "{BASE_PROJECT_CONFIG}\n[approval]\nauto_approve = [\"edit\"]\n"
    ));

    let error = resolve(&fixture.request()).expect_err("legacy authority must fail closed");
    match error {
        ConfigError::InvalidValue { source, message } => {
            assert_eq!(source.key, "approval.auto_approve");
            assert!(message.contains("[[approval.auto]]"), "{message}");
            assert!(message.contains("prepared-call"), "{message}");
            assert!(!message.contains("[\"edit\"]"), "{message}");
        }
        other => panic!("expected a migration diagnostic, got {other:?}"),
    }
}

#[test]
fn empty_legacy_auto_approval_remains_inert_during_migration() {
    let fixture = Fixture::new();
    fixture.write_project(&format!(
        "{BASE_PROJECT_CONFIG}\n[approval]\nauto_approve = []\n"
    ));

    let resolution = resolve(&fixture.request()).expect("an empty legacy list is inert");
    assert_eq!(
        resolution
            .config
            .approval
            .auto_approve
            .expect("the empty source remains explainable")
            .value,
        Vec::<String>::new()
    );
}

#[test]
fn typed_auto_approval_rule_resolves_as_one_provenance_bound_grant() {
    let fixture = Fixture::new();
    fixture.write_user(
        r#"
[[approval.auto]]
revision = 1
tool = "smith/edit"
operations = ["replace", "create"]
permissions = ["fs.read", "fs.write", "fs.create"]
max_risk = "medium"
mount = "workspace"
paths = ["src/**", "tests/*.rs"]
expires_at = "2099-01-01T00:00:00Z"
max_uses = 7
"#,
    );
    fixture.write_project(BASE_PROJECT_CONFIG);

    let resolution = resolve(&fixture.request()).expect("a valid scoped rule");
    let rule = resolution.config.approval.auto.first().expect("one rule");
    assert_eq!(rule.source.layer, Layer::UserFile);
    assert_eq!(rule.source.key, "approval.auto[0]");
    assert_eq!(rule.value.revision, 1);
    assert_eq!(rule.value.tool, "smith/edit");
    assert_eq!(
        rule.value.operations,
        vec![
            AutoApprovalOperation::Replace,
            AutoApprovalOperation::Create
        ]
    );
    assert_eq!(
        rule.value.permissions,
        vec![
            AutoApprovalPermission::FsRead,
            AutoApprovalPermission::FsWrite,
            AutoApprovalPermission::FsCreate,
        ]
    );
    assert_eq!(rule.value.max_risk, AutoApprovalRisk::Medium);
    assert_eq!(rule.value.mount, AutoApprovalMount::Workspace);
    assert_eq!(rule.value.max_uses, Some(7));
    assert!(rule.value.expires_at_unix_ms.is_some());
}

#[test]
fn scoped_auto_approval_rejects_unknown_revisions_and_unsafe_paths() {
    for (revision, path, expected) in [
        (2, "src/**", "revision 2"),
        (1, "../other/**", "project-relative"),
        (1, "[", "invalid"),
    ] {
        let fixture = Fixture::new();
        fixture.write_user(&format!(
            r#"
[[approval.auto]]
revision = {revision}
tool = "smith/edit"
operations = ["replace"]
permissions = ["fs.read", "fs.write"]
max_risk = "medium"
mount = "workspace"
paths = ["{path}"]
"#,
        ));
        fixture.write_project(BASE_PROJECT_CONFIG);

        let error = resolve(&fixture.request()).expect_err("the rule must fail closed");
        let message = error.to_string();
        assert!(message.contains(expected), "{message}");
    }
}

#[test]
fn scoped_auto_approval_schema_rejects_unknown_fields() {
    let fixture = Fixture::new();
    fixture.write_user(
        r#"
[[approval.auto]]
revision = 1
tool = "smith/edit"
operations = ["replace"]
permissions = ["fs.read", "fs.write"]
max_risk = "medium"
mount = "workspace"
paths = ["src/**"]
surprise = true
"#,
    );
    fixture.write_project(BASE_PROJECT_CONFIG);

    let message = resolve(&fixture.request())
        .expect_err("unknown rule fields must fail closed")
        .to_string();
    assert!(message.contains("surprise"), "{message}");
}
