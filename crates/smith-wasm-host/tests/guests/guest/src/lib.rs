use smith_module_sdk::bindings::smith::module::types::*;
use smith_module_sdk::{
    CompletionRequestBuilder, Module, NoticeBuilder, StatusItemBuilder, ToolSpecBuilder,
    WatcherBuilder, host,
};
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
fn completion() -> CompletionRequest {
    CompletionRequestBuilder::new(32)
        .system("fixture system")
        .message(Role::User, "fixture prompt")
        .build()
}
fn refuse_model_phase() {
    assert!(matches!(
        host::complete(&completion()),
        Err(HostError::NotAvailableInPhase)
    ));
}
impl Module for Fixture {
    fn describe() -> Descriptor {
        if cfg!(feature = "complete") {
            refuse_model_phase();
        }
        Descriptor {
            id: "guest".into(),
            revision: "1".into(),
            description: "Conformance guest".into(),
            default_enabled: true,
            requirements: vec![],
        }
    }
    fn mount(_: MountContext) -> Result<Mounted, ModuleError> {
        if cfg!(feature = "complete") {
            refuse_model_phase();
        }
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
        if cfg!(feature = "watcher")
            || cfg!(feature = "poll-oversize")
            || cfg!(feature = "poll-many")
            || cfg!(feature = "poll-unknown")
            || cfg!(feature = "poll-loop")
            || cfg!(feature = "watcher-duplicate")
            || cfg!(feature = "watcher-invalid")
        {
            let id = if cfg!(feature = "watcher-invalid") {
                "runtime.core.watch"
            } else if cfg!(feature = "poll-unknown") {
                "unknown"
            } else {
                "watch"
            };
            let mut declarations = vec![WatcherBuilder::new(id).build()];
            if cfg!(feature = "watcher-duplicate") {
                declarations.push(WatcherBuilder::new(id).build());
            }
            return Ok(Mounted::Contributions(declarations));
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
        if cfg!(feature = "complete") {
            return Ok(Mounted::Contributions(vec![
                tool("probe"),
                Contribution::StatusItem("count".into()),
                Contribution::ContextContributor(ComponentSpec {
                    id: "model-context".into(),
                    ..component.clone()
                }),
                Contribution::ToolOutputProcessor(ComponentSpec {
                    id: "model-output".into(),
                    ..component
                }),
                Contribution::Watcher("watch".into()),
            ]));
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
        if cfg!(feature = "complete") {
            let mut request = completion();
            if input.arguments_json.contains("output-limit") {
                request.max_output_tokens = 4097;
            }
            if input.arguments_json.contains("many-messages") {
                request.messages = (0..65)
                    .map(|_| CompletionMessage {
                        role: Role::User,
                        text: "prompt".into(),
                    })
                    .collect();
            }
            if input.arguments_json.contains("large-input") {
                request.system = Some("x".repeat(1024 * 1024 + 1));
            }
            let result = host::complete(&request);
            if input.arguments_json.contains("propagate") && result.is_err() {
                return Err(error());
            }
            if input.arguments_json.contains("loop-after-complete") {
                loop {
                    std::hint::spin_loop();
                }
            }
            return Ok(outcome(&format!("{result:?}")));
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
        if cfg!(feature = "complete") {
            refuse_model_phase();
            return Ok(ContextPatch { fragments: vec![] });
        }
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
        if cfg!(feature = "complete") {
            refuse_model_phase();
            return Ok(ToolOutputPatch {
                outcome_json: view.outcome_json,
            });
        }
        Ok(ToolOutputPatch {
            outcome_json: if view.call.arguments_json.contains("malformed") {
                "invalid".into()
            } else {
                outcome("processed")
            },
        })
    }
    fn poll(id: String) -> Result<Vec<Notice>, ModuleError> {
        if cfg!(feature = "complete") {
            refuse_model_phase();
        }
        if id != "watch" {
            return Err(ModuleError {
                kind: ErrorKind::UnknownId,
                message: "unknown watcher".into(),
            });
        }
        if cfg!(feature = "poll-loop") {
            loop {
                std::hint::spin_loop();
            }
        }
        if cfg!(feature = "poll-oversize") {
            return Ok(vec![NoticeBuilder::new("x".repeat(4097)).build()]);
        }
        if cfg!(feature = "poll-many") {
            return Ok((0..9)
                .map(|_| NoticeBuilder::new("notice").build())
                .collect());
        }
        Ok(vec![NoticeBuilder::new("fixture notice").build()])
    }
    fn status(id: String) -> Option<StatusItem> {
        if cfg!(feature = "complete") {
            refuse_model_phase();
        }
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
