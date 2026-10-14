//! The small CLI grammar deliberately validates the declaration Smith passes.

use anyhow::{Result, bail, ensure};
use serde::Serialize;
use serde_json::{Value, json};

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "kebab-case")]
pub(crate) enum Dialect {
    ClaudeCode,
    Codex,
}

impl Dialect {
    pub(crate) fn detect(args: &[String]) -> Result<Self> {
        if args.first().is_some_and(|arg| arg == "exec") {
            Ok(Self::Codex)
        } else if args.iter().any(|arg| arg == "-p") {
            Ok(Self::ClaudeCode)
        } else {
            bail!("expected exec (Codex) or -p (Claude Code)")
        }
    }
}

#[derive(Clone, Debug, Serialize)]
pub(crate) struct Server {
    pub(crate) name: String,
    pub(crate) command: String,
    pub(crate) args: Vec<String>,
    pub(crate) default_tools_approval_mode: Option<String>,
    pub(crate) tool_timeout_sec: Option<i64>,
    pub(crate) required: Option<bool>,
    pub(crate) startup_timeout_sec: Option<i64>,
}

#[derive(Debug, Serialize)]
pub(crate) struct Invocation {
    pub(crate) server: Option<Server>,
    pub(crate) prompt: String,
    pub(crate) resume: Option<String>,
    pub(crate) flags: Value,
}

pub(crate) const RESUME_REJECTED: &str = include_str!(
    "../../../smith-runtime/tests/fixtures/cli_bridge/codex-0.162.1/stderr/exec-flags-after-resume-rejected.txt"
);

impl Invocation {
    pub(crate) fn parse(args: &[String], dialect: Dialect) -> Result<Self> {
        let mut invocation = Self {
            server: None,
            prompt: String::new(),
            resume: None,
            flags: json!({}),
        };
        let mut servers = serde_json::Map::new();
        let mut index = usize::from(dialect == Dialect::Codex);
        let mut resumed = false;
        while index < args.len() {
            let arg = &args[index];
            if dialect == Dialect::Codex
                && resumed
                && matches!(arg.as_str(), "-C" | "--sandbox" | "-c")
            {
                bail!("{RESUME_REJECTED}");
            }
            match arg.as_str() {
                "--verbose" if dialect == Dialect::ClaudeCode => {
                    invocation.flags[arg] = json!(true);
                }
                "--json" | "--skip-git-repo-check" if dialect == Dialect::Codex => {
                    invocation.flags[arg] = json!(true);
                }
                "resume" if dialect == Dialect::Codex => {
                    ensure!(!resumed, "duplicate resume subcommand");
                    invocation.resume = Some(next(args, &mut index, arg)?.to_owned());
                    resumed = true;
                }
                "-p" if dialect == Dialect::ClaudeCode => {
                    invocation.prompt = next(args, &mut index, arg)?.to_owned();
                }
                "--mcp-config" if dialect == Dialect::ClaudeCode => {
                    let config = next(args, &mut index, arg)?;
                    let value: Value = serde_json::from_str(config)
                        .map_err(|error| anyhow::anyhow!("--mcp-config: {error}"))?;
                    servers = value["mcpServers"]
                        .as_object()
                        .ok_or_else(|| anyhow::anyhow!("--mcp-config: expected mcpServers object"))?
                        .clone();
                }
                "-c" if dialect == Dialect::Codex => {
                    let config = next(args, &mut index, arg)?;
                    parse_override(config, &mut servers)
                        .map_err(|error| anyhow::anyhow!("-c {config}: {error}"))?;
                }
                "--resume"
                | "--allowedTools"
                | "--permission-mode"
                | "--model"
                | "--output-format"
                | "--append-system-prompt"
                    if dialect == Dialect::ClaudeCode =>
                {
                    let value = next(args, &mut index, arg)?;
                    invocation.flags[arg] = json!(value);
                    if arg == "--resume" {
                        invocation.resume = Some(value.to_owned());
                    }
                }
                "--sandbox" | "-C" | "-m" if dialect == Dialect::Codex => {
                    invocation.flags[arg] = json!(next(args, &mut index, arg)?);
                }
                _ if dialect == Dialect::Codex && !arg.starts_with('-') => {
                    ensure!(
                        index == args.len() - 1,
                        "unexpected positional argument {arg}"
                    );
                    invocation.prompt = arg.to_owned();
                }
                _ => bail!("unexpected argument {arg}"),
            }
            index += 1;
        }
        ensure!(
            servers.len() <= 1,
            "expected at most one MCP server declaration"
        );
        if let Some((name, declaration)) = servers.into_iter().next() {
            let command = declaration["command"].as_str().ok_or_else(|| {
                anyhow::anyhow!("MCP declaration (--mcp-config/-c): missing command")
            })?;
            let args = declaration["args"]
                .as_array()
                .ok_or_else(|| anyhow::anyhow!("MCP declaration (--mcp-config/-c): missing args"))?
                .iter()
                .map(|value| {
                    value.as_str().map(str::to_owned).ok_or_else(|| {
                        anyhow::anyhow!("MCP declaration (--mcp-config/-c): args must be strings")
                    })
                })
                .collect::<Result<Vec<_>>>()?;
            invocation.server = Some(Server {
                name,
                command: command.to_owned(),
                args,
                default_tools_approval_mode: declaration["default_tools_approval_mode"]
                    .as_str()
                    .map(str::to_owned),
                tool_timeout_sec: declaration["tool_timeout_sec"].as_i64(),
                required: declaration["required"].as_bool(),
                startup_timeout_sec: declaration["startup_timeout_sec"].as_i64(),
            });
        }
        Ok(invocation)
    }

    pub(crate) fn allowed(&self, dialect: Dialect) -> bool {
        let Some(server) = &self.server else {
            return false;
        };
        match dialect {
            Dialect::ClaudeCode => self.flags["--allowedTools"].as_str().is_some_and(|rules| {
                rules
                    .split(|c: char| c == ',' || c.is_whitespace())
                    .any(|rule| rule == format!("mcp__{}", server.name))
            }),
            Dialect::Codex => server.default_tools_approval_mode.as_deref() == Some("approve"),
        }
    }
}

fn next<'a>(args: &'a [String], index: &mut usize, flag: &str) -> Result<&'a str> {
    *index += 1;
    args.get(*index)
        .map(String::as_str)
        .ok_or_else(|| anyhow::anyhow!("{flag}: missing value"))
}

fn parse_override(config: &str, servers: &mut serde_json::Map<String, Value>) -> Result<()> {
    let (key, input) = config
        .split_once('=')
        .ok_or_else(|| anyhow::anyhow!("expected key=value"))?;
    let mut parts = key.split('.');
    ensure!(
        parts.next() == Some("mcp_servers"),
        "expected mcp_servers.<name>.<field>"
    );
    let name = parts
        .next()
        .filter(|name| !name.is_empty())
        .ok_or_else(|| anyhow::anyhow!("missing server name"))?;
    let field = parts
        .next()
        .ok_or_else(|| anyhow::anyhow!("missing field"))?;
    ensure!(parts.next().is_none(), "unexpected key component");
    let input = input.trim();
    // The existing TOML dev-dependency decodes escapes correctly, including
    // Unicode escapes; this guard limits the accepted grammar to CLI overrides.
    match field {
        "command" | "default_tools_approval_mode" => {
            ensure!(
                basic_string_end(input)? == input.len(),
                "extra text after TOML string"
            );
        }
        "args" => basic_string_array(input)?,
        "tool_timeout_sec" | "startup_timeout_sec" => {
            ensure!(input.parse::<i64>().is_ok(), "expected integer");
        }
        "required" => ensure!(matches!(input, "true" | "false"), "expected boolean"),
        _ => bail!("unsupported field {field}"),
    }
    let table: toml::Table = format!("value={input}").parse()?;
    ensure!(table.len() == 1, "extra TOML fields");
    let value = &table["value"];
    let decoded = match field {
        "command" | "default_tools_approval_mode" => json!(
            value
                .as_str()
                .ok_or_else(|| anyhow::anyhow!("expected string"))?
        ),
        "args" => {
            let strings = value
                .as_array()
                .ok_or_else(|| anyhow::anyhow!("expected array"))?
                .iter()
                .map(|v| {
                    v.as_str()
                        .ok_or_else(|| anyhow::anyhow!("expected string array"))
                })
                .collect::<Result<Vec<_>>>()?;
            json!(strings)
        }
        "required" => json!(
            value
                .as_bool()
                .ok_or_else(|| anyhow::anyhow!("expected boolean"))?
        ),
        _ => json!(
            value
                .as_integer()
                .ok_or_else(|| anyhow::anyhow!("expected integer"))?
        ),
    };
    servers.entry(name.to_owned()).or_insert_with(|| json!({}))[field] = decoded;
    Ok(())
}

fn basic_string_end(input: &str) -> Result<usize> {
    ensure!(
        input.starts_with('"') && !input.starts_with("\"\"\""),
        "expected TOML basic string"
    );
    let mut escaped = false;
    for (index, character) in input.char_indices().skip(1) {
        ensure!(
            !matches!(character, '\n' | '\r'),
            "expected a single-line TOML basic string"
        );
        if escaped {
            escaped = false;
        } else if character == '\\' {
            escaped = true;
        } else if character == '"' {
            return Ok(index + 1);
        }
    }
    bail!("unterminated TOML basic string")
}

fn basic_string_array(input: &str) -> Result<()> {
    let mut rest = input
        .strip_prefix('[')
        .ok_or_else(|| anyhow::anyhow!("expected TOML array of basic strings"))?
        .trim();
    loop {
        if let Some(end) = rest.strip_prefix(']') {
            ensure!(end.trim().is_empty(), "extra text after TOML array");
            return Ok(());
        }
        let length = basic_string_end(rest)?;
        rest = rest[length..].trim();
        if rest.starts_with(']') {
            continue;
        }
        rest = rest
            .strip_prefix(',')
            .ok_or_else(|| anyhow::anyhow!("expected comma between basic strings"))?
            .trim();
    }
}
