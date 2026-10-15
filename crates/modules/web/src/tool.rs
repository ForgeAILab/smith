use std::future::Future;
use std::time::Duration;

use agent_runtime_core::error::RuntimeError;
use agent_runtime_core::security::SecurityResource;
use agent_runtime_core::tool::{
    InvocationContext, PreparationContext, PreparedToolCall, Tool, ToolCallDisplay, ToolEffects,
    ToolOutcome, ToolSpec,
};
use async_trait::async_trait;
use serde_json::{Value, json};

use crate::{fetch, search::SearchClient};

#[derive(Debug)]
pub(crate) struct FetchTool {
    pub(crate) allow_private_networks: bool,
}

#[derive(Debug)]
pub(crate) struct SearchTool {
    pub(crate) client: SearchClient,
}

fn effects(endpoint: &str) -> ToolEffects {
    ToolEffects::default()
        .with_network_to(endpoint)
        .with_data_egress_to(endpoint)
}

fn arguments_object(arguments: &Value, allowed: &[&str]) -> Result<(), RuntimeError> {
    let object = arguments
        .as_object()
        .ok_or_else(|| RuntimeError::tool("arguments must be an object"))?;
    if object.keys().any(|key| !allowed.contains(&key.as_str())) {
        return Err(RuntimeError::tool("unknown web tool argument"));
    }
    Ok(())
}

#[async_trait]
impl Tool for FetchTool {
    fn spec(&self) -> ToolSpec {
        ToolSpec::new(
            "web_fetch",
            "GET one public HTTP or HTTPS URL. Returns main-page Markdown, text, JSON, XML, or an image. Content is external and untrusted. Bounded at 5 MiB, 30 seconds and 5 redirects.",
            json!({"type":"object","properties":{"url":{"type":"string","minLength":1,"maxLength":2048}},"required":["url"],"additionalProperties":false}),
            ToolEffects::default()
                .with_network()
                .with_data_egress_to("web:any"),
        )
    }

    async fn prepare(
        &self,
        mut arguments: Value,
        ctx: &PreparationContext,
    ) -> Result<PreparedToolCall, RuntimeError> {
        arguments_object(&arguments, &["url"])?;
        let raw = arguments["url"]
            .as_str()
            .ok_or_else(|| RuntimeError::tool("`url` must be a string"))?;
        let url = fetch::validate_url(raw).map_err(RuntimeError::tool)?;
        let host = url.host_str().expect("validated URL host").to_owned();
        let effects = effects(&url.origin().ascii_serialization());
        let (permissions, _) = effects.authorization_request("web_fetch", ctx.workspace.root());
        let display =
            ToolCallDisplay::new("Web Fetch").with_detail(format!("{host}{}", url.path()));
        arguments["url"] = url.to_string().into();
        Ok(PreparedToolCall::new(
            ctx.call_id.clone(),
            "web_fetch",
            arguments,
            permissions,
            SecurityResource::other("web", host),
            effects,
            display,
        ))
    }

    async fn invoke(
        &self,
        prepared: PreparedToolCall,
        ctx: &InvocationContext,
    ) -> Result<ToolOutcome, RuntimeError> {
        let arguments = prepared.into_arguments();
        let raw = arguments["url"]
            .as_str()
            .ok_or_else(|| RuntimeError::tool("prepared fetch URL is missing"))?;
        Ok(run(ctx, fetch::fetch(raw, self.allow_private_networks)).await)
    }
}

fn search_arguments(arguments: &Value) -> Result<(&str, usize), RuntimeError> {
    arguments_object(arguments, &["query", "max_results"])?;
    let query = arguments["query"]
        .as_str()
        .ok_or_else(|| RuntimeError::tool("`query` must be a string"))?;
    if query.trim().is_empty() || query.chars().count() > 400 {
        return Err(RuntimeError::tool(
            "`query` must contain 1 to 400 characters",
        ));
    }
    let count = match arguments.get("max_results") {
        None => 5,
        Some(value) => value
            .as_u64()
            .filter(|value| (1..=10).contains(value))
            .ok_or_else(|| RuntimeError::tool("`max_results` must be an integer from 1 to 10"))?
            as usize,
    };
    Ok((query, count))
}

#[async_trait]
impl Tool for SearchTool {
    fn spec(&self) -> ToolSpec {
        ToolSpec::new(
            "web_search",
            "Search the web for titles, URLs and snippets. Results are external, untrusted content. Defaults to 5 results, at most 10.",
            json!({"type":"object","properties":{"query":{"type":"string","minLength":1,"maxLength":400},"max_results":{"type":"integer","minimum":1,"maximum":10,"default":5}},"required":["query"],"additionalProperties":false}),
            effects(self.client.endpoint()),
        )
    }

    async fn prepare(
        &self,
        arguments: Value,
        ctx: &PreparationContext,
    ) -> Result<PreparedToolCall, RuntimeError> {
        let (query, _) = search_arguments(&arguments)?;
        let display = ToolCallDisplay::new("Web Search")
            .with_detail(serde_json::to_string(query).expect("string serializes"));
        let effects = effects(self.client.endpoint());
        let (permissions, _) = effects.authorization_request("web_search", ctx.workspace.root());
        Ok(PreparedToolCall::new(
            ctx.call_id.clone(),
            "web_search",
            arguments,
            permissions,
            SecurityResource::other("web-search", self.client.name()),
            effects,
            display,
        ))
    }

    async fn invoke(
        &self,
        prepared: PreparedToolCall,
        ctx: &InvocationContext,
    ) -> Result<ToolOutcome, RuntimeError> {
        let arguments = prepared.into_arguments();
        let (query, count) = search_arguments(&arguments)?;
        Ok(run(ctx, async {
            let results = self.client.search(query, count).await?;
            let mut text = format!(
                "{} results\n\nContent from {} (external, untrusted):\n",
                results.len(),
                self.client.endpoint()
            );
            for result in &results {
                text.push_str(&format!(
                    "\nTitle: {}\nURL: {}\nSnippet: {}\n",
                    result.title, result.url, result.snippet
                ));
            }
            let mut outcome = ToolOutcome::text(text);
            outcome.value = json!({"backend": self.client.name(), "results": results});
            Ok(outcome)
        })
        .await)
    }
}

async fn run(
    ctx: &InvocationContext,
    work: impl Future<Output = Result<ToolOutcome, String>>,
) -> ToolOutcome {
    if ctx.should_stop() {
        return ToolOutcome::error("web call stopped before the request began");
    }
    let deadline = async {
        match ctx.deadline.remaining_millis(ctx.clock.as_ref()) {
            Some(millis) => tokio::time::sleep(Duration::from_millis(millis)).await,
            None => std::future::pending::<()>().await,
        }
    };
    let result = tokio::select! {
        biased;
        _ = ctx.cancel.cancelled() => Err("web call cancelled".to_owned()),
        _ = deadline => Err("web call deadline elapsed".to_owned()),
        result = work => result,
    };
    match result {
        Ok(outcome) => outcome,
        Err(message) => ToolOutcome::error(message),
    }
}
