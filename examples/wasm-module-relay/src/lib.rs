//! Relay: a mailbox shared by every Smith session of one user.
//!
//! The host offers no directory listing, no delete and no atomic rename, so the
//! layout is flat and every file except `peers` has exactly one writer:
//!
//! - `peers`                 newline-separated names (re-added by its owner if lost)
//! - `sess.<session>`        the name a session answers to
//! - `owner.<name>`          the session holding a name
//! - `msg.<from>.<to>.<n>`   one message body, written before its counter moves
//! - `out.<from>.<to>`       messages sent so far, owned by the sender
//! - `ack.<to>.<from>`       messages read so far, owned by the recipient
use std::sync::Mutex;

use smith_module_sdk::bindings::smith::module::types::*;
use smith_module_sdk::{
    ContextPatchBuilder, Module, NoticeBuilder, StatusItemBuilder, ToolSpecBuilder, WatcherBuilder,
    host,
};

struct Relay;
smith_module_sdk::export!(Relay);

const MAX_NAME: usize = 24;
const MAX_PEERS: usize = 64;
const MAX_TEXT: usize = 8 * 1024;
const MAX_INBOX: usize = 20;
const STATUS: &str = "relay";
const NOTICE: &str = "unread";
const WATCHER: &str = "mailbox";
static ANNOUNCED: Mutex<String> = Mutex::new(String::new());

/// Name and unread count last seen by this instance; `status` has no session.
static SEEN: Mutex<Option<(String, usize)>> = Mutex::new(None);

fn error(kind: ErrorKind, message: impl Into<String>) -> ModuleError {
    ModuleError {
        kind,
        message: message.into(),
    }
}

fn unknown() -> ModuleError {
    error(ErrorKind::UnknownId, "unknown contribution")
}

fn file(path: &str) -> FilePath {
    FilePath {
        root: "plugin-data".into(),
        path: path.into(),
    }
}

fn read(path: &str) -> Option<String> {
    String::from_utf8(host::fs_read(&file(path)).ok()?).ok()
}

fn write(path: &str, text: &str) -> Result<(), ModuleError> {
    host::fs_write(&FileWrite {
        file: file(path),
        bytes: text.as_bytes().to_vec(),
    })
    .map_err(|_| error(ErrorKind::Failed, "the relay mailbox could not be written"))
}

/// A half-written counter reads as zero; its owner rewrites it on the next send.
fn count(path: &str) -> u64 {
    read(path)
        .and_then(|text| text.trim().parse().ok())
        .unwrap_or(0)
}

fn valid_name(name: &str) -> bool {
    !name.is_empty()
        && name.len() <= MAX_NAME
        && name
            .bytes()
            .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'-')
}

fn session_key(session: &str) -> String {
    session
        .chars()
        .filter(|c| c.is_ascii_alphanumeric() || *c == '-' || *c == '_')
        .take(96)
        .collect()
}

fn peers() -> Vec<String> {
    read("peers")
        .unwrap_or_default()
        .lines()
        .filter(|name| valid_name(name))
        .map(str::to_owned)
        .take(MAX_PEERS)
        .collect()
}

fn claim(session: &str, name: &str) -> Result<(), ModuleError> {
    write(&format!("owner.{name}"), session)?;
    write(&format!("sess.{}", session_key(session)), name)
}

/// The session's name, registering a default on first use.
fn identity(session: &str) -> Result<String, ModuleError> {
    let name = match read(&format!("sess.{}", session_key(session))) {
        Some(name) if valid_name(name.trim()) => name.trim().to_owned(),
        _ => {
            // Session ids share a prefix; their tail is what tells them apart.
            let tail: Vec<_> = session
                .chars()
                .rev()
                .filter(char::is_ascii_alphanumeric)
                .map(|c| c.to_ascii_lowercase())
                .take(8)
                .collect();
            let short: String = tail.into_iter().rev().collect();
            let name = format!("s-{short}");
            claim(session, &name)?;
            name
        }
    };
    let mut listed = peers();
    if !listed.contains(&name) {
        if listed.len() >= MAX_PEERS {
            return Err(error(ErrorKind::Unavailable, "the relay peer list is full"));
        }
        listed.push(name.clone());
        write("peers", &listed.join("\n"))?;
    }
    Ok(name)
}

/// Unread counts for `me`, one entry per sender with something waiting.
fn waiting(me: &str) -> Vec<(String, u64, u64)> {
    peers()
        .into_iter()
        .filter(|peer| peer != me)
        .filter_map(|peer| {
            let sent = count(&format!("out.{peer}.{me}"));
            let read = count(&format!("ack.{me}.{peer}"));
            (sent > read).then_some((peer, read, sent))
        })
        .collect()
}

fn remember(name: &str, unread: usize) {
    *SEEN.lock().unwrap_or_else(|e| e.into_inner()) = Some((name.to_owned(), unread));
}

fn reply(text: impl Into<String>, is_error: bool) -> String {
    serde_json::json!({
        "value": null,
        "content": [{ "type": "text", "text": text.into() }],
        "is_error": is_error,
    })
    .to_string()
}

fn argument(arguments: &serde_json::Value, key: &str) -> Result<String, String> {
    arguments
        .get(key)
        .and_then(|value| value.as_str())
        .map(str::to_owned)
        .ok_or_else(|| format!("`{key}` must be a string"))
}

fn join(session: &str, arguments: &serde_json::Value) -> Result<String, ModuleError> {
    let name = match argument(arguments, "name") {
        Ok(name) => name,
        Err(reason) => return Ok(reply(reason, true)),
    };
    if !valid_name(&name) {
        return Ok(reply(
            format!("a name is 1-{MAX_NAME} characters of a-z, 0-9 and `-`"),
            true,
        ));
    }
    let previous = identity(session)?;
    if let Some(owner) = read(&format!("owner.{name}"))
        && !owner.is_empty()
        && owner != session
    {
        return Ok(reply(format!("`{name}` is held by another session"), true));
    }
    claim(session, &name)?;
    let mut listed: Vec<_> = peers()
        .into_iter()
        .filter(|peer| *peer != previous)
        .collect();
    if !listed.contains(&name) {
        listed.push(name.clone());
    }
    write("peers", &listed.join("\n"))?;
    if previous != name {
        write(&format!("owner.{previous}"), "")?;
    }
    remember(&name, waiting(&name).len());
    Ok(reply(format!("this session is now `{name}`"), false))
}

fn list(session: &str) -> Result<String, ModuleError> {
    let me = identity(session)?;
    let others: Vec<_> = peers().into_iter().filter(|peer| *peer != me).collect();
    Ok(reply(
        if others.is_empty() {
            format!("you are `{me}`; no other session has joined the relay")
        } else {
            format!("you are `{me}`; peers: {}", others.join(", "))
        },
        false,
    ))
}

fn send(session: &str, arguments: &serde_json::Value) -> Result<String, ModuleError> {
    let (to, text) = match (argument(arguments, "to"), argument(arguments, "text")) {
        (Ok(to), Ok(text)) => (to, text),
        (Err(reason), _) | (_, Err(reason)) => return Ok(reply(reason, true)),
    };
    if text.is_empty() || text.len() > MAX_TEXT {
        return Ok(reply(
            format!("a message is 1-{MAX_TEXT} bytes of text"),
            true,
        ));
    }
    let me = identity(session)?;
    let known = peers();
    let recipients: Vec<_> = if to == "*" {
        known.into_iter().filter(|peer| *peer != me).collect()
    } else if to == me {
        return Ok(reply("a session cannot message itself", true));
    } else if known.contains(&to) {
        vec![to]
    } else {
        return Ok(reply(
            format!("no session is named `{to}`; list peers first"),
            true,
        ));
    };
    if recipients.is_empty() {
        return Ok(reply("no other session has joined the relay", true));
    }
    for peer in &recipients {
        let next = count(&format!("out.{me}.{peer}")) + 1;
        write(&format!("msg.{me}.{peer}.{next}"), &text)?;
        write(&format!("out.{me}.{peer}"), &next.to_string())?;
    }
    Ok(reply(
        format!(
            "sent to {}; its live session will be notified by the relay watcher",
            recipients.join(", ")
        ),
        false,
    ))
}

fn inbox(session: &str) -> Result<String, ModuleError> {
    let me = identity(session)?;
    let mut lines = Vec::new();
    let mut left = 0;
    for (peer, read_so_far, sent) in waiting(&me) {
        let mut last = read_so_far;
        for n in read_so_far + 1..=sent {
            if lines.len() == MAX_INBOX {
                left += sent - last;
                break;
            }
            // A missing body means the sender's counter outran a failed write.
            let body = read(&format!("msg.{peer}.{me}.{n}"))
                .unwrap_or_else(|| "(message body unavailable)".into());
            lines.push(format!("from {peer}: {body}"));
            last = n;
        }
        if last > read_so_far {
            write(&format!("ack.{me}.{peer}"), &last.to_string())?;
        }
    }
    remember(&me, left as usize);
    Ok(reply(
        match (lines.is_empty(), left) {
            (true, _) => format!("no unread messages for `{me}`"),
            (false, 0) => lines.join("\n\n"),
            (false, _) => format!("{}\n\n{left} more unread; call again", lines.join("\n\n")),
        },
        false,
    ))
}

fn tool(name: &str, description: &str, schema: &str, effect: Effect) -> Contribution {
    Contribution::Tool(
        ToolSpecBuilder::new(name, description, schema)
            .effect(effect)
            .build(),
    )
}

impl Module for Relay {
    fn describe() -> Descriptor {
        Descriptor {
            id: "relay".into(),
            revision: "1".into(),
            description: "Message other Smith sessions on this machine".into(),
            default_enabled: true,
            requirements: vec![],
        }
    }

    fn mount(_: MountContext) -> Result<Mounted, ModuleError> {
        let reads = || Effect::HostRead("plugin-data".into());
        let writes = || {
            Effect::HostWrite(HostWriteEffect {
                resource_kind: "plugin-data".into(),
                scope: "relay".into(),
            })
        };
        Ok(Mounted::Contributions(vec![
            tool(
                "join",
                "Choose the name other Smith sessions use to message this one. \
                 Without it the session answers to a generated name.",
                r#"{"type":"object","properties":{"name":{"type":"string","description":"1-24 characters of a-z, 0-9 and -"}},"required":["name"],"additionalProperties":false}"#,
                writes(),
            ),
            tool(
                "peers",
                "Show this session's relay name and the other sessions that can be messaged.",
                r#"{"type":"object","properties":{},"additionalProperties":false}"#,
                reads(),
            ),
            tool(
                "send",
                "Send a text message to another Smith session by relay name, or to every \
                 peer with `*`. The relay watcher notifies the live recipient.",
                r#"{"type":"object","properties":{"to":{"type":"string","description":"A peer name, or * for every peer"},"text":{"type":"string"}},"required":["to","text"],"additionalProperties":false}"#,
                writes(),
            ),
            tool(
                "inbox",
                "Read and mark as read the messages other Smith sessions sent to this one.",
                r#"{"type":"object","properties":{},"additionalProperties":false}"#,
                writes(),
            ),
            Contribution::ContextContributor(ComponentSpec {
                id: NOTICE.into(),
                revision: "1".into(),
                before: vec![],
                after: vec![],
            }),
            Contribution::StatusItem(STATUS.into()),
            WatcherBuilder::new(WATCHER).build(),
        ]))
    }

    fn call_tool(id: String, input: ToolInput) -> Result<String, ModuleError> {
        let arguments = serde_json::from_str(&input.arguments_json)
            .map_err(|_| error(ErrorKind::InvalidInput, "arguments are not JSON"))?;
        match id.as_str() {
            "join" => join(&input.session, &arguments),
            "peers" => list(&input.session),
            "send" => send(&input.session, &arguments),
            "inbox" => inbox(&input.session),
            _ => Err(unknown()),
        }
    }

    /// Announces waiting mail without consuming it: a fragment is rebuilt for
    /// every request, so only the `inbox` tool result may move the read cursor.
    fn contribute_context(id: String, view: ContextView) -> Result<ContextPatch, ModuleError> {
        if id != NOTICE {
            return Err(unknown());
        }
        let me = identity(&view.session)?;
        let pending = waiting(&me);
        let total: u64 = pending.iter().map(|(_, read, sent)| sent - read).sum();
        remember(&me, total as usize);
        if pending.is_empty() {
            return Ok(ContextPatchBuilder::default().build());
        }
        let senders: Vec<_> = pending.iter().map(|(peer, ..)| peer.as_str()).collect();
        Ok(ContextPatchBuilder::default()
            .fragment(ContextFragment {
                id: NOTICE.into(),
                kind: FragmentKind::Memory,
                position: ContextPosition {
                    lane: ContextLane::Memory,
                    sequence: 0,
                },
                source: FragmentSource::Host,
                revision: total.to_string(),
                requirement: Requirement::Optional,
                priority: 0,
                content: FragmentContent::Text(format!(
                    "Relay: this session is `{me}` and has {total} unread message(s) from \
                     other Smith sessions ({}). Read them with the relay inbox tool.",
                    senders.join(", ")
                )),
                pairing: None,
                pairings: vec![],
                conversation_group: None,
                sensitivity: Sensitivity::Internal,
                cache_class: CacheClass::Ephemeral,
                token_hint: None,
            })
            .build())
    }

    fn poll(id: String) -> Result<Vec<Notice>, ModuleError> {
        if id != WATCHER {
            return Err(unknown());
        }
        // A root owns its own instance. A tool or context call registers its
        // mailbox before polling; poll needs no history authority.
        let Some((me, _)) = SEEN.lock().unwrap_or_else(|e| e.into_inner()).clone() else {
            return Ok(vec![]);
        };
        let pending = waiting(&me);
        let total: u64 = pending.iter().map(|(_, read, sent)| sent - read).sum();
        remember(&me, total as usize);
        let signature = format!(
            "{me}:{:?}",
            pending
                .iter()
                .map(|(peer, _, sent)| (peer, sent))
                .collect::<Vec<_>>()
        );
        let mut announced = ANNOUNCED.lock().unwrap_or_else(|e| e.into_inner());
        if *announced == signature {
            return Ok(vec![]);
        }
        *announced = signature;
        if total == 0 {
            return Ok(vec![]);
        }
        let senders = pending
            .iter()
            .map(|(peer, _, _)| peer.as_str())
            .collect::<Vec<_>>()
            .join(", ");
        Ok(vec![NoticeBuilder::new(format!(
            "Relay: `{me}` has {total} unread message(s) from {senders}. Read them with the relay inbox tool."
        )).build()])
    }

    fn process_tool_output(_: String, _: ToolOutputView) -> Result<ToolOutputPatch, ModuleError> {
        Err(unknown())
    }

    fn status(id: String) -> Option<StatusItem> {
        if id != STATUS {
            return None;
        }
        let seen = SEEN.lock().unwrap_or_else(|e| e.into_inner()).clone();
        Some(match seen {
            None => StatusItemBuilder::new(id, "relay ready").build(),
            Some((name, 0)) => StatusItemBuilder::new(id, format!("relay {name}")).build(),
            Some((name, unread)) => {
                StatusItemBuilder::new(id, format!("relay {name} · {unread} unread"))
                    .severity(StatusSeverity::Warning)
                    .build()
            }
        })
    }
}
