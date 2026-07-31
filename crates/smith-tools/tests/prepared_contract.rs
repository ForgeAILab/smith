//! Contract tests for the immutable action Smith authorizes before invocation.

use std::sync::Arc;

use agent_runtime_core::cancel::Cancellation;
use agent_runtime_core::clock::{Deadline, SystemClock};
use agent_runtime_core::ids::{RequestId, SessionId, ToolCallId, TurnId};
use agent_runtime_core::security::{PermissionSet, SecurityResource};
use agent_runtime_core::tool::{PreparationContext, Tool};
use agent_runtime_core::workspace::Workspace;
use agent_runtime_registry::Permission;
use serde_json::json;
use smith_host::workspace::ProjectWorkspace;
use smith_tools::{EditTool, ListTool, ReadTool, SearchTool, ShellTool};

struct Project {
    _dir: tempfile::TempDir,
    root: String,
    preparation: PreparationContext,
}

impl Project {
    fn new() -> Self {
        let dir = tempfile::tempdir().expect("a temporary project");
        std::fs::create_dir_all(dir.path().join("src")).expect("an existing source directory");
        std::fs::write(dir.path().join("src/lib.rs"), "pub fn old() {}\n")
            .expect("an existing source file");
        let workspace = Arc::new(ProjectWorkspace::new(dir.path()).expect("a project workspace"));
        let root = workspace.root().to_owned();
        Self {
            _dir: dir,
            root,
            preparation: PreparationContext {
                session: SessionId::new("session-1"),
                turn: Some(TurnId::new("turn-1")),
                call_id: ToolCallId::new("call-1"),
                request: RequestId::new("request-1"),
                workspace,
                clock: Arc::new(SystemClock),
                cancel: Cancellation::new(),
                deadline: Deadline::never(),
            },
        }
    }

    fn path(&self, relative: &str) -> String {
        std::path::Path::new(&self.root)
            .join(relative)
            .to_string_lossy()
            .into_owned()
    }

    fn resource(&self, segments: &[&str]) -> SecurityResource {
        SecurityResource::filesystem(
            self.root.clone(),
            segments
                .iter()
                .map(|segment| (*segment).to_owned())
                .collect(),
        )
    }
}

fn permissions(values: impl IntoIterator<Item = Permission>) -> PermissionSet {
    values.into_iter().collect()
}

#[tokio::test]
async fn read_shaped_tools_prepare_canonical_exact_read_resources() {
    let project = Project::new();
    let cases: Vec<(Box<dyn Tool>, serde_json::Value, String, SecurityResource)> = vec![
        (
            Box::new(ReadTool),
            json!({"path": "src/lib.rs"}),
            project.path("src/lib.rs"),
            project.resource(&["src", "lib.rs"]),
        ),
        (
            Box::new(ListTool),
            json!({"path": "src"}),
            project.path("src"),
            project.resource(&["src"]),
        ),
        (
            Box::new(SearchTool),
            json!({"pattern": "old", "path": "src"}),
            project.path("src"),
            project.resource(&["src"]),
        ),
    ];

    for (tool, arguments, canonical_path, resource) in cases {
        let spec = tool.spec();
        let prepared = tool
            .prepare(arguments, &project.preparation)
            .await
            .expect("a prepared read");

        assert_eq!(
            prepared.arguments()["path"],
            canonical_path,
            "{}",
            spec.name
        );
        assert_eq!(prepared.resource(), &resource, "{}", spec.name);
        assert_eq!(
            prepared.required_permissions(),
            &PermissionSet::single(Permission::FsRead),
            "{}",
            spec.name
        );
        assert!(prepared.effects().is_read_only(), "{}", spec.name);
        assert!(
            prepared
                .required_permissions()
                .is_subset(&spec.permission_upper_bound),
            "{} exceeded its advertised permission upper bound",
            spec.name
        );
    }
}

#[tokio::test]
async fn edit_prepares_distinct_exact_contracts_for_replace_and_create() {
    let project = Project::new();
    let target = project.path("src/lib.rs");
    let resource = project.resource(&["src", "lib.rs"]);

    let replacement = EditTool
        .prepare(
            json!({
                "path": "src/lib.rs",
                "old_string": "old",
                "new_string": "new",
            }),
            &project.preparation,
        )
        .await
        .expect("a prepared replacement");
    assert_eq!(replacement.arguments()["path"], target);
    assert_eq!(replacement.resource(), &resource);
    assert_eq!(
        replacement.required_permissions(),
        &permissions([Permission::FsRead, Permission::FsWrite])
    );
    assert!(replacement.effects().has_read());
    assert_eq!(
        replacement
            .effects()
            .write_scopes()
            .map(|scope| scope.as_str())
            .collect::<Vec<_>>(),
        [target.as_str()]
    );

    let created_target = project.path("src/new.rs");
    let created = EditTool
        .prepare(
            json!({
                "path": "src/new.rs",
                "old_string": "",
                "new_string": "pub fn new() {}\n",
            }),
            &project.preparation,
        )
        .await
        .expect("a prepared creation");
    assert_eq!(created.arguments()["path"], created_target);
    assert_eq!(created.resource(), &project.resource(&["src", "new.rs"]));
    assert_eq!(
        created.required_permissions(),
        &permissions([Permission::FsWrite, Permission::FsCreate])
    );
    assert!(!created.effects().has_read());
    assert_eq!(
        created
            .effects()
            .write_scopes()
            .map(|scope| scope.as_str())
            .collect::<Vec<_>>(),
        [created_target.as_str()]
    );

    let bound = EditTool.spec().permission_upper_bound;
    assert!(replacement.required_permissions().is_subset(&bound));
    assert!(created.required_permissions().is_subset(&bound));
}

#[tokio::test]
async fn shell_prepares_broad_workspace_authority_even_for_a_nested_cwd() {
    let project = Project::new();
    let prepared = ShellTool
        .prepare(
            json!({
                "command": "printf ready",
                "cwd": "src",
                "timeout_ms": 900_000,
            }),
            &project.preparation,
        )
        .await
        .expect("a prepared shell command");

    let required = permissions([
        Permission::FsRead,
        Permission::FsWrite,
        Permission::FsCreate,
        Permission::FsDelete,
        Permission::ProcessSpawn,
        Permission::NetHttp,
        Permission::DataEgress,
    ]);
    assert_eq!(prepared.arguments()["cwd"], project.path("src"));
    assert_eq!(prepared.arguments()["timeout_ms"], 600_000);
    assert_eq!(
        prepared.resource(),
        &SecurityResource::filesystem(project.root.clone(), Vec::new())
    );
    assert_eq!(prepared.required_permissions(), &required);
    assert_eq!(
        prepared
            .effects()
            .write_scopes()
            .map(|scope| scope.as_str())
            .collect::<Vec<_>>(),
        [project.root.as_str()]
    );
    assert!(prepared.effects().has_read());
    assert!(prepared.effects().spawns_process());
    assert!(prepared.effects().has_network());
    assert!(
        prepared
            .required_permissions()
            .is_subset(&ShellTool.spec().permission_upper_bound)
    );
}
