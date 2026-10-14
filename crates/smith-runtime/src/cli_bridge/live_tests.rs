//! Opt-in socket server for the orchestrator's installed-CLI smoke.

use std::fs::{self, OpenOptions};
use std::io::Write;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use agent_runtime_core::{
    content::ToolResultBlock,
    error::RuntimeError,
    tool::{ToolEffects, ToolSpec},
};
use async_trait::async_trait;
use serde_json::{Value, json};

use super::BridgedSet;
use super::endpoint::{BridgeDispatch, BridgeEndpoint};
use super::endpoint_tests::result;

struct LiveDispatch {
    calls: Mutex<fs::File>,
}

#[async_trait]
impl BridgeDispatch for LiveDispatch {
    async fn call(&self, name: String, arguments: Value) -> Result<ToolResultBlock, RuntimeError> {
        {
            let mut calls = self.calls.lock().unwrap();
            writeln!(calls, "{}", json!({"name":name,"arguments":arguments}))
                .map_err(|error| RuntimeError::tool(format!("live call log: {error}")))?;
            calls
                .flush()
                .map_err(|error| RuntimeError::tool(format!("live call log: {error}")))?;
        }
        let text = match name.as_str() {
            "echo" => format!(
                "ECHO:{}",
                arguments["text"]
                    .as_str()
                    .ok_or_else(|| RuntimeError::tool("text must be a string"))?
            ),
            "wait" => {
                let seconds = arguments["seconds"]
                    .as_f64()
                    .ok_or_else(|| RuntimeError::tool("seconds must be a number"))?;
                let duration = Duration::try_from_secs_f64(seconds)
                    .map_err(|error| RuntimeError::tool(format!("invalid seconds: {error}")))?;
                tokio::time::sleep(duration).await;
                "DONE".into()
            }
            _ => return Err(RuntimeError::tool("unknown live tool")),
        };
        Ok(result(&name, &text))
    }
}

fn environment_path(name: &str) -> PathBuf {
    std::env::var_os(name)
        .map(PathBuf::from)
        .unwrap_or_else(|| panic!("set {name} for the live helper"))
}

#[tokio::test]
#[ignore = "orchestrator smoke with installed CLIs; requires SMITH_BRIDGE_LIVE_*_FILE paths"]
async fn live_bridge_endpoint() {
    tokio::time::timeout(Duration::from_secs(610), async {
        let socket_file = environment_path("SMITH_BRIDGE_LIVE_SOCKET_FILE");
        let calls_file = environment_path("SMITH_BRIDGE_LIVE_CALLS_FILE");
        let stop_file = environment_path("SMITH_BRIDGE_LIVE_STOP_FILE");
        let calls = OpenOptions::new().create(true).append(true).open(calls_file).unwrap();
        let dispatch = Arc::new(LiveDispatch { calls: Mutex::new(calls) });
        let set = BridgedSet::new(vec![
            ToolSpec::new("echo", "Returns text prefixed with ECHO:",
                json!({"type":"object","properties":{"text":{"type":"string"}},"required":["text"]}), ToolEffects::read_only()),
            ToolSpec::new("wait", "Sleeps seconds, then returns DONE",
                json!({"type":"object","properties":{"seconds":{"type":"number"}},"required":["seconds"]}), ToolEffects::read_only()),
        ], false);
        let endpoint = BridgeEndpoint::bind_test(set, dispatch, Path::new("/tmp"),
            Arc::new(|| rustix::process::getuid().as_raw()),
            Some("SMITH-BRIDGE-MARKER: the bridge marker word is KUMQUAT-4471.".into())).unwrap();
        let directory = endpoint.socket_path().parent().unwrap().to_owned();
        fs::write(socket_file, endpoint.socket_path().as_os_str().as_encoded_bytes()).unwrap();
        let end = tokio::time::Instant::now() + Duration::from_secs(600);
        while !stop_file.exists() && tokio::time::Instant::now() < end {
            tokio::time::sleep(Duration::from_millis(100)).await;
        }
        endpoint.close().await;
        assert!(!directory.exists());
    }).await.expect("live bridge deadline");
}
