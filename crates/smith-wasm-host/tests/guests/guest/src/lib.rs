use smith_module_sdk::bindings::smith::module::types::*;
use smith_module_sdk::{Module, StatusItemBuilder, ToolSpecBuilder, host};
struct Fixture;
static CALLS: std::sync::atomic::AtomicU32 = std::sync::atomic::AtomicU32::new(0);
smith_module_sdk::export!(Fixture);
fn error() -> ModuleError {
    ModuleError {
        kind: ErrorKind::Failed,
        message: "fixture error".into(),
    }
}
fn outcome(text: &str) -> String {
    // Test strings are ASCII; escape quotes/backslashes for the exact ToolOutcome.
    let text = text.replace('\\', "\\\\").replace('"', "\\\"");
    format!(
        "{{\"value\":null,\"content\":[{{\"type\":\"text\",\"text\":\"{text}\"}}],\"is_error\":false}}"
    )
}
impl Module for Fixture {
    fn describe() -> Descriptor {
        Descriptor {
            id: "guest".into(),
            revision: "1".into(),
            description: "Conformance guest".into(),
            default_enabled: true,
            requirements: vec![],
        }
    }
    fn mount(_: MountContext) -> Result<Mounted, ModuleError> {
        if cfg!(feature = "mount-http") {
            host::http(&HttpRequest {
                method: HttpMethod::Post,
                url: "https://api.example.com/".into(),
                headers: vec![],
                body: vec![],
            })
            .map_err(|_| error())?;
        }
        if cfg!(feature = "inactive") {
            return Ok(Mounted::Inactive("fixture inactive".into()));
        }
        if cfg!(feature = "mount-error") {
            return Err(error());
        }
        let component = ComponentSpec {
            id: "context".into(),
            revision: "1".into(),
            before: vec![],
            after: vec![],
        };
        if cfg!(feature = "context") {
            return Ok(Mounted::Contributions(vec![
                Contribution::ContextContributor(component),
            ]));
        }
        if cfg!(feature = "processor") {
            return Ok(Mounted::Contributions(vec![
                Contribution::ToolOutputProcessor(component),
            ]));
        }
        let tool = |name: &str| {
            Contribution::Tool(
                ToolSpecBuilder::new(name, "Fixture tool", "{\"type\":\"object\"}")
                    .effect(Effect::Read)
                    .permission(Permission::Other("fixture".into()))
                    .build(),
            )
        };
        if cfg!(feature = "context-tools") {
            return Ok(Mounted::Contributions(vec![
                tool("probe"),
                Contribution::ContextContributor(component),
            ]));
        }
        if cfg!(feature = "malformed") {
            return Ok(Mounted::Contributions(vec![tool("probe"), tool("probe")]));
        }
        if cfg!(feature = "collision") {
            return Ok(Mounted::Contributions(vec![tool("read")]));
        }
        Ok(Mounted::Contributions(vec![
            tool("probe"),
            tool("second"),
            Contribution::StatusItem("count".into()),
        ]))
    }
    fn call_tool(_: String, input: ToolInput) -> Result<String, ModuleError> {
        CALLS.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
        if cfg!(feature = "quiet") {
            return Ok(outcome("ok"));
        }
        let action = input.arguments_json;
        if action.contains("trap") {
            panic!("fixture trap");
        }
        if action.contains("loop") {
            loop {
                std::hint::spin_loop();
            }
        }
        if action.contains("memory") {
            let mut bytes = Vec::new();
            loop {
                bytes.extend(vec![42u8; 1024 * 1024]);
                std::hint::black_box(&bytes);
            }
        }
        if action.contains("oversized") {
            return Ok(outcome(&"x".repeat(1024 * 1024 + 1)));
        }
        if action.contains("malformed") {
            return Ok("invalid".into());
        }
        if action.contains("http") {
            let url = if action.contains("other") {
                "https://other.example.net/"
            } else {
                "https://api.example.com/"
            };
            return Ok(outcome(&format!(
                "{:?}",
                host::http(&HttpRequest {
                    method: HttpMethod::Post,
                    url: url.into(),
                    headers: vec![],
                    body: vec![]
                })
            )));
        }
        if action.contains("cache-write") {
            return Ok(outcome(&format!(
                "{:?}",
                host::fs_write(&FileWrite {
                    file: FilePath {
                        root: "workspace".into(),
                        path: "host/plugins/wasm-cache/poison".into()
                    },
                    bytes: b"poison".to_vec()
                })
            )));
        }
        if action.contains("fs-read") {
            return Ok(outcome(&format!(
                "{:?}",
                host::fs_read(&FilePath {
                    root: "workspace".into(),
                    path: "fixture.txt".into()
                })
            )));
        }
        if action.contains("fs-write") {
            return Ok(outcome(&format!(
                "{:?}",
                host::fs_write(&FileWrite {
                    file: FilePath {
                        root: "workspace".into(),
                        path: "fixture.txt".into()
                    },
                    bytes: b"written".to_vec()
                })
            )));
        }
        if action.contains("kv-set") {
            return Ok(outcome(&format!(
                "{:?}",
                host::kv_set(&KvEntry {
                    key: "fixture".into(),
                    value: b"value".to_vec()
                })
            )));
        }
        if action.contains("kv-get") {
            return Ok(outcome(&format!("{:?}", host::kv_get("fixture"))));
        }
        if action.contains("history") {
            return Ok(outcome(&format!("{:?}", host::session_history(10))));
        }
        host::log(LogLevel::Info, "fixture log").map_err(|_| error())?;
        Ok(outcome("ok"))
    }
    fn contribute_context(_: String, view: ContextView) -> Result<ContextPatch, ModuleError> {
        let malformed = view.history.iter().any(|m| {
            m.content.iter().any(
                |p| matches!(p, ContentPart::Leaf(LeafContent::Text(t)) if t.contains("malformed")),
            )
        });
        Ok(ContextPatch {
            fragments: vec![ContextFragment {
                id: "fragment".into(),
                kind: if malformed {
                    FragmentKind::History
                } else {
                    FragmentKind::Memory
                },
                position: ContextPosition {
                    lane: ContextLane::Memory,
                    sequence: 0,
                },
                source: FragmentSource::Host,
                revision: "1".into(),
                requirement: Requirement::Optional,
                priority: 0,
                content: FragmentContent::Text(if cfg!(feature = "context-tools") {
                    format!("history: {}", view.history.len())
                } else {
                    "guest context".into()
                }),
                pairing: None,
                pairings: vec![],
                conversation_group: None,
                sensitivity: Sensitivity::Internal,
                cache_class: CacheClass::Ephemeral,
                token_hint: None,
            }],
        })
    }
    fn process_tool_output(
        _: String,
        view: ToolOutputView,
    ) -> Result<ToolOutputPatch, ModuleError> {
        Ok(ToolOutputPatch {
            outcome_json: if view.call.arguments_json.contains("malformed") {
                "invalid".into()
            } else {
                outcome("processed")
            },
        })
    }
    fn status(id: String) -> Option<StatusItem> {
        Some(
            StatusItemBuilder::new(
                id,
                format!(
                    "calls: {}",
                    CALLS.load(std::sync::atomic::Ordering::Relaxed)
                ),
            )
            .build(),
        )
    }
}
