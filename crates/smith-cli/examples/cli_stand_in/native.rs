//! Deterministic native provider for comparing the two dispatch paths.
use std::fs::OpenOptions;
use std::io::{self, BufRead, Write};

use anyhow::{Result, ensure};
use serde_json::{Value, json};

/// Answer compatibility probes before collecting argv or starting fixture work.
pub(crate) fn probe() -> Option<Result<i32>> {
    let mut args = std::env::args().skip(1);
    if args.next().as_deref() != Some("--native-contract-provider")
        || args.nth(2).as_deref() != Some("--smith-provider-probe")
    {
        return None;
    }
    Some((|| {
        let model = args
            .next()
            .ok_or_else(|| anyhow::anyhow!("native provider probe needs a model"))?;
        // Only the model needs JSON escaping. No config, record, call file,
        // request log, stdin, environment inventory, or JSON tree is needed.
        let mut out = io::stdout().lock();
        out.write_all(b"{\"protocol\":\"smith-command-provider\",\"schema_version\":1,\"model\":")?;
        serde_json::to_writer(&mut out, &model)?;
        out.write_all(
            b",\"implementation\":\"native-contract\",\"implementation_version\":\"1.0.0\"}\n",
        )?;
        out.flush()?;
        Ok(0)
    })())
}

pub(crate) fn serve(args: &[String]) -> Result<i32> {
    ensure!(
        args.len() >= 4,
        "native provider needs a call file and request log"
    );
    let emit = |mut value: Value| -> Result<()> {
        value["protocol"] = json!("smith-command-provider");
        value["schema_version"] = json!(1);
        let mut out = io::stdout().lock();
        serde_json::to_writer(&mut out, &value)?;
        writeln!(out)?;
        out.flush()?;
        Ok(())
    };
    ensure!(
        args[3] == "--smith-provider-attempt",
        "unknown native provider invocation"
    );
    let mut line = String::new();
    io::stdin().lock().read_line(&mut line)?;
    let request: Value = serde_json::from_str(&line)?;
    let mut log = OpenOptions::new()
        .create(true)
        .append(true)
        .open(&args[2])?;
    serde_json::to_writer(&mut log, &request)?;
    writeln!(log)?;
    log.flush()?;
    let call: Value = serde_json::from_slice(&std::fs::read(&args[1])?)?;
    if let Some(answer) = call["answer"].as_str() {
        // Advisor contracts use the same recorder without asking the advisor
        // to dispatch a tool of its own.
        emit(json!({"type":"text_delta", "text":answer}))?;
        emit(json!({"type":"usage", "input_tokens":6, "output_tokens":3}))?;
        emit(json!({"type":"finish", "reason":"stop"}))?;
        return Ok(0);
    }
    let tool = call["tool"]
        .as_str()
        .ok_or_else(|| anyhow::anyhow!("native tool name"))?;
    let parts = request["request"]["messages"]
        .as_array()
        .expect("native messages")
        .iter()
        .flat_map(|message| message["content"].as_array().expect("message parts"))
        .collect::<Vec<_>>();
    let result = parts
        .iter()
        .rev()
        .find(|part| part["type"] == "tool_result" && part["name"] == tool);
    let next = if let Some(result) = result {
        // Consume an allowed spawn before completing the parent, so parity
        // does not depend on automatic child-completion scheduling.
        let text = result["content"]
            .as_array()
            .expect("result content")
            .iter()
            .filter_map(|part| part["text"].as_str())
            .collect::<Vec<_>>()
            .join("\n");
        let value: Value = serde_json::from_str(&text).unwrap_or(Value::Null);
        if let Some(child) = value["spawned"].as_str()
            && !parts
                .iter()
                .any(|part| part["type"] == "tool_call" && part["arguments"]["action"] == "wait")
        {
            Some((tool.to_owned(), json!({"action":"wait", "child_id":child})))
        } else if tool == "agent"
            && result["is_error"] == true
            && !parts
                .iter()
                .any(|part| part["type"] == "tool_call" && part["arguments"]["action"] == "list")
        {
            Some((tool.to_owned(), json!({"action":"list"})))
        } else {
            None
        }
    } else if request["request"]["tools"]
        .as_array()
        .expect("native tools")
        .iter()
        .any(|spec| spec["name"] == tool)
    {
        Some((tool.to_owned(), call["arguments"].clone()))
    } else {
        // Native activation is budgeted; request the target if it was not
        // preactivated. A rejected activation must fail promptly, not loop.
        ensure!(
            !parts
                .iter()
                .any(|part| part["type"] == "tool_call" && part["name"] == "registry.activate"),
            "native tool never became available: {request}"
        );
        Some((
            "registry.activate".to_owned(),
            json!({"ids":[format!("tool:{tool}")]}),
        ))
    };
    if let Some((name, arguments)) = &next {
        emit(
            json!({"type":"tool_call_delta", "index":0, "id":format!("native-{}", parts.len()),
            "name":name, "arguments_fragment":arguments.to_string()}),
        )?;
    } else {
        emit(json!({"type":"text_delta", "text":"native contract complete"}))?;
    }
    emit(json!({"type":"usage", "input_tokens":6, "output_tokens":3}))?;
    emit(json!({"type":"finish", "reason":if next.is_some() {"tool_calls"} else {"stop"}}))?;
    Ok(0)
}
