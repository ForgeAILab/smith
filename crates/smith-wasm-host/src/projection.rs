use crate::WasmError;
use crate::bindings::smith::module::types as w;
use agent_runtime::context::ContextFragment;
use agent_runtime_core::content::{ContentPart, Message};
use agent_runtime_core::store::VersionedSessionState;
use serde::{Serialize, de::DeserializeOwned};

fn malformed() -> WasmError {
    WasmError::MalformedResult("unrepresentable protocol value".into())
}
fn camel(s: &str) -> String {
    s.split('_')
        .map(|part| {
            let mut chars = part.chars();
            chars.next().map_or_else(String::new, |c| {
                c.to_uppercase().collect::<String>() + chars.as_str()
            })
        })
        .collect()
}
fn snake(s: &str) -> String {
    let mut out = String::new();
    for (i, c) in s.chars().enumerate() {
        if c.is_uppercase() && i > 0 {
            out.push('_');
        }
        out.extend(c.to_lowercase());
    }
    out
}
pub(crate) fn enum_to_wit<T: Serialize, U: DeserializeOwned>(value: T) -> Result<U, WasmError> {
    let json = serde_json::to_value(value).map_err(|_| malformed())?;
    let text = json.as_str().ok_or_else(malformed)?;
    serde_json::from_value(serde_json::Value::String(camel(text)))
        .map_err(|_| WasmError::MalformedResult(format!("unsupported protocol enum value {text}")))
}
pub(crate) fn capability(c: smith_module::ModuleCapability) -> w::Capability {
    use smith_module::ModuleCapability as C;
    match c {
        C::WorkspaceRead => w::Capability::WorkspaceRead,
        C::WorkspaceWrite => w::Capability::WorkspaceWrite,
        C::HostRead => w::Capability::HostRead,
        C::HostWrite => w::Capability::HostWrite,
        C::Process => w::Capability::Process,
        C::Network => w::Capability::Network,
        C::ExternalRead => w::Capability::ExternalRead,
        C::ExternalWrite => w::Capability::ExternalWrite,
        C::DataEgress => w::Capability::DataEgress,
        C::Credential => w::Capability::Credential,
        C::Approval => w::Capability::Approval,
        C::Provider => w::Capability::Provider,
        C::Model => w::Capability::Model,
        C::SessionNotify => w::Capability::SessionNotify,
    }
}
pub(crate) fn messages(messages: &[Message]) -> Result<Vec<w::Message>, WasmError> {
    messages
        .iter()
        .map(|m| {
            Ok(w::Message {
                role: enum_to_wit(m.role)?,
                content: m.content.iter().map(part).collect::<Result<_, _>>()?,
            })
        })
        .collect()
}
fn part(p: &ContentPart) -> Result<w::ContentPart, WasmError> {
    match p {
        ContentPart::ToolResult(r) => Ok(w::ContentPart::ToolResult(w::ToolResult {
            call_id: r.call_id.to_string(),
            name: r.name.clone(),
            content: r.content.iter().map(leaf).collect::<Result<_, _>>()?,
            is_error: r.is_error,
        })),
        _ => Ok(w::ContentPart::Leaf(leaf(p)?)),
    }
}
fn leaf(p: &ContentPart) -> Result<w::LeafContent, WasmError> {
    Ok(match p {
        ContentPart::Text { text } => w::LeafContent::Text(text.clone()),
        ContentPart::Reasoning {
            text,
            redacted,
            signature,
            producer,
        } => w::LeafContent::Reasoning(w::Reasoning {
            text: if *redacted {
                String::new()
            } else {
                text.clone()
            },
            redacted: *redacted,
            signature: if *redacted { None } else { signature.clone() },
            producer: producer.as_ref().map(|p| w::ReasoningProducer {
                provider: p.provider.clone(),
                model: p.model.to_string(),
            }),
        }),
        ContentPart::Image { url, detail } => w::LeafContent::Image(w::Image {
            url: url.clone(),
            detail: detail.clone(),
        }),
        ContentPart::ToolCall(call) => w::LeafContent::ToolCall(w::ToolCall {
            id: call.id.to_string(),
            name: call.name.clone(),
            arguments_json: call.arguments.to_string(),
        }),
        ContentPart::ToolResult(_) => return Err(malformed()),
    })
}
pub(crate) fn state(
    state: &Option<VersionedSessionState>,
) -> Result<Option<w::ComponentState>, WasmError> {
    state
        .as_ref()
        .map(|s| {
            let mut nodes = vec![];
            tree(&s.value, &mut nodes, 0)?;
            Ok(w::ComponentState {
                revision: s.revision.as_str().into(),
                sensitivity: enum_to_wit(s.sensitivity)?,
                value: w::ValueTree { nodes },
            })
        })
        .transpose()
}
fn tree(
    value: &serde_json::Value,
    nodes: &mut Vec<w::ValueNode>,
    depth: usize,
) -> Result<u32, WasmError> {
    if depth > 32 || nodes.len() >= 4096 {
        return Err(WasmError::OversizedPayload("state tree".into()));
    }
    let index = nodes.len() as u32;
    nodes.push(w::ValueNode::Null);
    nodes[index as usize] = match value {
        serde_json::Value::Null => w::ValueNode::Null,
        serde_json::Value::Bool(b) => w::ValueNode::Boolean(*b),
        serde_json::Value::Number(n) => {
            if let Some(n) = n.as_i64() {
                w::ValueNode::Signed(n)
            } else if let Some(n) = n.as_u64() {
                w::ValueNode::Unsigned(n)
            } else {
                w::ValueNode::Float(n.as_f64().ok_or_else(malformed)?)
            }
        }
        serde_json::Value::String(s) => w::ValueNode::Text(s.clone()),
        serde_json::Value::Array(values) => w::ValueNode::Array(
            values
                .iter()
                .map(|v| tree(v, nodes, depth + 1))
                .collect::<Result<_, _>>()?,
        ),
        serde_json::Value::Object(values) => w::ValueNode::Object(
            values
                .iter()
                .map(|(key, v)| {
                    Ok(w::ValueMember {
                        key: key.clone(),
                        node: tree(v, nodes, depth + 1)?,
                    })
                })
                .collect::<Result<_, WasmError>>()?,
        ),
    };
    Ok(index)
}
pub(crate) fn fragments(
    patch: w::ContextPatch,
    namespace: &str,
) -> Result<Vec<ContextFragment>, WasmError> {
    let mut ids = std::collections::BTreeSet::new();
    patch
        .fragments
        .into_iter()
        .map(|fragment| {
            crate::limits::identifier(&fragment.id)?;
            crate::limits::revision(&fragment.revision)?;
            if !ids.insert(fragment.id.clone()) {
                return Err(malformed());
            }
            let valid = matches!(
                (&fragment.kind, &fragment.position.lane),
                (
                    w::FragmentKind::SystemInstruction | w::FragmentKind::DeveloperInstruction,
                    w::ContextLane::Instructions
                ) | (
                    w::FragmentKind::Memory | w::FragmentKind::Retrieval,
                    w::ContextLane::Memory
                ) | (w::FragmentKind::Continuation, w::ContextLane::TailContext)
            );
            if !valid
                || !matches!(fragment.source, w::FragmentSource::Host)
                || fragment.pairing.is_some()
                || !fragment.pairings.is_empty()
                || fragment.conversation_group.is_some()
            {
                return Err(WasmError::MalformedResult(
                    "protected fragment placement/authority".into(),
                ));
            }
            let w::FragmentContent::Text(text) = &fragment.content else {
                return Err(malformed());
            };
            let mut json = serde_json::to_value(&fragment).map_err(|_| malformed())?;
            json["id"] = serde_json::json!(format!("{namespace}.{}", fragment.id));
            for field in ["kind", "requirement", "sensitivity", "cache_class"] {
                let value = json[field].as_str().ok_or_else(malformed)?;
                json[field] = serde_json::json!(snake(value));
            }
            let lane = json["position"]["lane"].as_str().ok_or_else(malformed)?;
            json["position"]["lane"] = serde_json::json!(snake(lane));
            json["source"] = serde_json::json!({"source":"host"});
            json["content"] = serde_json::json!({"content":"text", "value":text});
            serde_json::from_value(json).map_err(|_| malformed())
        })
        .collect()
}
pub(crate) fn usage_counters(
    delta: &agent_runtime_core::usage::UsageDelta,
) -> Result<Vec<w::UsageCounter>, WasmError> {
    delta
        .iter()
        .map(|(kind, value)| {
            Ok(w::UsageCounter {
                kind: enum_to_wit(kind)?,
                value,
            })
        })
        .collect()
}

pub(crate) fn usage(
    records: &[agent_runtime_core::usage::UsageRecord],
) -> Result<Vec<w::UsageRecord>, WasmError> {
    records
        .iter()
        .map(|record| {
            let p = &record.provenance;
            Ok(w::UsageRecord {
                source: enum_to_wit(record.source)?,
                provenance: w::UsageProvenance {
                    request: p.request.as_ref().map(ToString::to_string),
                    attempt: p.attempt.as_ref().map(ToString::to_string),
                    tool_call: p.tool_call.as_ref().map(ToString::to_string),
                    purpose: p.purpose.clone(),
                    attempt_purpose: p.attempt_purpose.map(enum_to_wit).transpose()?,
                    cache_identity_digest: p
                        .cache_identity
                        .as_ref()
                        .map(|c| c.digest().as_str().into()),
                    failed: p.failed,
                },
                counters: record
                    .delta
                    .iter()
                    .map(|(kind, value)| {
                        Ok(w::UsageCounter {
                            kind: enum_to_wit(kind)?,
                            value,
                        })
                    })
                    .collect::<Result<_, WasmError>>()?,
            })
        })
        .collect()
}
pub(crate) fn tool_spec(
    tool: &w::ToolSpec,
    namespace: &str,
) -> Result<agent_runtime_core::tool::ToolSpec, WasmError> {
    use agent_runtime_core::tool::ToolSpec;
    let schema: serde_json::Value = serde_json::from_str(&tool.parameters_json)
        .map_err(|_| WasmError::MalformedResult("invalid tool JSON Schema".into()))?;
    if !schema.is_object() || schema.get("type").and_then(|t| t.as_str()) != Some("object") {
        return Err(WasmError::MalformedResult(
            "tool schema must describe an object".into(),
        ));
    }
    local_schema(&schema)?;
    jsonschema::meta::validate(&schema)
        .map_err(|_| WasmError::MalformedResult("invalid tool JSON Schema".into()))?;
    // Use the upstream unreviewed MCP classifier, not guest effects or
    // permission claims. Adding Read to the external floor would select a
    // filesystem resource and fail validation before reaching approval.
    let policy = agent_runtime_mcp::McpServerConfig::stdio(namespace, format!("wasm:{namespace}"));
    Ok(ToolSpec::new(
        format!("{namespace}__{}", tool.name),
        &tool.description,
        schema,
        policy.resolved_effect_floor(),
    ))
}

// Schemas are guest data, never an ambient filesystem/network retrieval API.
fn local_schema(value: &serde_json::Value) -> Result<(), WasmError> {
    match value {
        serde_json::Value::Object(fields) => {
            for (key, value) in fields {
                if matches!(
                    key.as_str(),
                    "$ref" | "$dynamicRef" | "$recursiveRef" | "$id"
                ) && value.as_str().is_some_and(|v| !v.starts_with('#'))
                {
                    return Err(WasmError::MalformedResult(
                        "tool schemas must be self-contained (references are unsupported)".into(),
                    ));
                }
                if key == "$schema"
                    && value.as_str().is_some_and(|v| {
                        ![
                            "http://json-schema.org/draft-04/schema#",
                            "http://json-schema.org/draft-06/schema#",
                            "http://json-schema.org/draft-07/schema#",
                            "https://json-schema.org/draft/2019-09/schema",
                            "https://json-schema.org/draft/2020-12/schema",
                        ]
                        .contains(&v)
                    })
                {
                    return Err(WasmError::MalformedResult(
                        "unsupported tool schema dialect".into(),
                    ));
                }
                if !matches!(key.as_str(), "const" | "enum" | "default" | "examples") {
                    local_schema(value)?;
                }
            }
        }
        serde_json::Value::Array(values) => {
            for value in values {
                local_schema(value)?;
            }
        }
        _ => {}
    }
    Ok(())
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn schemas_cannot_load_host_files_or_remote_resources() {
        for reference in [
            "file:///private/secret.json",
            "https://ungranted.example/schema.json",
        ] {
            assert!(local_schema(&serde_json::json!({"type":"object", "$ref":reference})).is_err());
        }
        assert!(
            local_schema(
                &serde_json::json!({"type":"object", "properties":{"id":{"type":"string"}}})
            )
            .is_ok()
        );
    }
}
