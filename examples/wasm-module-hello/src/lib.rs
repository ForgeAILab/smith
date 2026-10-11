use smith_module_sdk::bindings::smith::module::types::*;
use smith_module_sdk::{Module, StatusItemBuilder, ToolSpecBuilder, host};
struct Hello;
static CALLED: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);
smith_module_sdk::export!(Hello);
fn unknown() -> ModuleError {
    ModuleError {
        kind: ErrorKind::UnknownId,
        message: "unknown contribution".into(),
    }
}
impl Module for Hello {
    fn describe() -> Descriptor {
        Descriptor {
            id: "hello".into(),
            revision: "1".into(),
            description: "Read the workspace README".into(),
            default_enabled: true,
            requirements: vec![],
        }
    }
    fn mount(_: MountContext) -> Result<Mounted, ModuleError> {
        Ok(Mounted::Contributions(vec![
            Contribution::Tool(
                ToolSpecBuilder::new(
                    "hello",
                    "Read README.md in the workspace",
                    r#"{"type":"object","properties":{},"additionalProperties":false}"#,
                )
                .effect(Effect::Read)
                .build(),
            ),
            Contribution::StatusItem("hello-status".into()),
        ]))
    }
    fn call_tool(id: String, _: ToolInput) -> Result<String, ModuleError> {
        if id != "hello" {
            return Err(unknown());
        }
        let bytes = host::fs_read(&FilePath {
            root: "workspace".into(),
            path: "README.md".into(),
        })
        .map_err(|_| ModuleError {
            kind: ErrorKind::Failed,
            message: "README.md could not be read through the workspace grant".into(),
        })?;
        CALLED.store(true, std::sync::atomic::Ordering::Relaxed);
        let label = format!("Hello: README.md contains {} bytes", bytes.len());
        Ok(format!(
            r#"{{"value":null,"content":[{{"type":"text","text":"{label}"}}],"is_error":false}}"#
        ))
    }
    fn contribute_context(_: String, _: ContextView) -> Result<ContextPatch, ModuleError> {
        Err(unknown())
    }
    fn process_tool_output(_: String, _: ToolOutputView) -> Result<ToolOutputPatch, ModuleError> {
        Err(unknown())
    }
    fn status(id: String) -> Option<StatusItem> {
        (id == "hello-status").then(|| {
            StatusItemBuilder::new(
                id,
                if CALLED.load(std::sync::atomic::Ordering::Relaxed) {
                    "hello called"
                } else {
                    "hello ready"
                },
            )
            .build()
        })
    }
}
