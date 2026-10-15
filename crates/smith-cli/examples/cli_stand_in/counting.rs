//! The append-only log makes an invocation observable independently of output.

use std::fs::{self, OpenOptions};
use std::io::{self, BufRead, Write};
use std::path::Path;
use std::thread;
use std::time::Duration;

use anyhow::{Result, bail};
use serde_json::{Value, json};

pub(crate) fn serve(args: &[String]) -> Result<()> {
    if !matches!(args.len(), 3 | 5)
        || args[1] != "--log"
        || (args.len() == 5 && args[3] != "--handshake-marker")
    {
        bail!("usage: --counting-mcp-server --log <file> [--handshake-marker <file>]");
    }
    let log = Path::new(&args[2]);
    let handshake_marker = args.get(4).map(Path::new);
    // The empty file is evidence that a refused call did not execute.
    OpenOptions::new().create(true).append(true).open(log)?;
    for line in io::stdin().lock().lines() {
        let request: Value = serde_json::from_str(&line?)?;
        let Some(id) = request.get("id") else {
            continue;
        };
        let method = request["method"].as_str().unwrap_or("");
        let response = match method {
            "initialize" => {
                // Startup alone creates the call log. The optional marker
                // distinguishes a CLI that actually negotiates MCP from one
                // that merely launches the server before rejecting its model.
                if let Some(marker) = handshake_marker {
                    fs::write(marker, serde_json::to_vec(&request)?)?;
                }
                json!({"result":{
                "protocolVersion":request["params"]["protocolVersion"],
                "capabilities":{"tools":{"listChanged":false}},
                "serverInfo":{"name":"counting","version":"1"},
                "instructions":"Count calls by reading the append-only invocation log."
                }})
            }
            "ping" => json!({"result":{}}),
            "tools/list" => json!({"result":{"tools":[
                {"name":"count","description":"Record arguments and count invocations.","inputSchema":{"type":"object"}},
                {"name":"slow_count","description":"Count after sleeping seconds.","inputSchema":{"type":"object","properties":{"seconds":{"type":"number"}}}}
            ]}}),
            "tools/call" => {
                let name = request["params"]["name"].as_str().unwrap_or("");
                let arguments = &request["params"]["arguments"];
                if !matches!(name, "count" | "slow_count") {
                    json!({"result":{"content":[{"type":"text","text":format!("unknown tool: {name}")}],"isError":true}})
                } else {
                    let mut started = OpenOptions::new()
                        .create(true)
                        .append(true)
                        .open(log.with_extension("started.jsonl"))?;
                    serde_json::to_writer(
                        &mut started,
                        &json!({"tool":name,"arguments":arguments,"pid":std::process::id(),
                        "authenticated":std::env::var("BRIDGE_TEST_TOKEN").ok().as_deref()
                            == Some("bridge-secret-must-not-reach-cli")}),
                    )?;
                    writeln!(started)?;
                    started.flush()?;
                    if name == "slow_count" {
                        let seconds = arguments["seconds"].as_f64().unwrap_or(0.0);
                        let duration = Duration::try_from_secs_f64(seconds)?;
                        thread::sleep(duration);
                    }
                    let mut file = OpenOptions::new().append(true).open(log)?;
                    serde_json::to_writer(&mut file, arguments)?;
                    writeln!(file)?;
                    file.flush()?;
                    let count = fs::read_to_string(log)?.lines().count();
                    let text = arguments["output"]
                        .as_str()
                        .map_or_else(|| format!("counted:{count}"), str::to_owned);
                    json!({"result":{"content":[{"type":"text","text":text}],"isError":false}})
                }
            }
            _ => json!({"error":{"code":-32601,"message":format!("method not found: {method}")}}),
        };
        let mut response = response;
        response["jsonrpc"] = json!("2.0");
        response["id"] = id.clone();
        let mut stdout = io::stdout().lock();
        serde_json::to_writer(&mut stdout, &response)?;
        writeln!(stdout)?;
        stdout.flush()?;
    }
    Ok(())
}
