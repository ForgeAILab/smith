use super::*;
/// Wraps built-in tools with mutation attribution.
pub fn observed_tools(recorder: Arc<ChangeRecorder>) -> Vec<Arc<dyn Tool>> {
    observe(Some(recorder), ReadRecorder::new())
}

/// Wraps built-in tools with mutation attribution and an explicit background
/// process owner supplied by the composing runtime.
pub fn observed_tools_with_background(
    recorder: Arc<ChangeRecorder>,
    background: Arc<dyn crate::background::BackgroundTaskHost>,
) -> Vec<Arc<dyn Tool>> {
    observe_with_background(Some(recorder), ReadRecorder::new(), background)
}

pub(crate) fn observe_with_background(
    recorder: Option<Arc<ChangeRecorder>>,
    reads: Arc<ReadRecorder>,
    background: Arc<dyn crate::background::BackgroundTaskHost>,
) -> Vec<Arc<dyn Tool>> {
    crate::built_in(background)
        .into_iter()
        .map(|inner| {
            Arc::new(ObservedTool {
                inner,
                recorder: recorder.clone(),
                reads: reads.clone(),
            }) as Arc<dyn Tool>
        })
        .collect()
}

#[derive(Debug)]
struct ObservedTool {
    inner: Arc<dyn Tool>,
    recorder: Option<Arc<ChangeRecorder>>,
    reads: Arc<ReadRecorder>,
}

impl ObservedTool {
    fn record(&self, mutation: ToolMutation) {
        if let Some(recorder) = &self.recorder {
            recorder.record(mutation);
        }
    }
}

#[async_trait]
impl Tool for ObservedTool {
    fn spec(&self) -> ToolSpec {
        self.inner.spec()
    }

    async fn prepare(
        &self,
        arguments: Value,
        ctx: &PreparationContext,
    ) -> Result<PreparedToolCall, RuntimeError> {
        let prepared = self.inner.prepare(arguments, ctx).await?;
        // Enforced after the inner prepare so the operation has been validated
        // and normalized, and so a doomed destructive call never reaches the
        // approval prompt. `edit` owns the rule; this supplies the state.
        if prepared.tool() == "edit" {
            match crate::edit::expected_read_version(prepared.arguments(), &self.reads) {
                Err(defect) => {
                    let display = prepared
                        .arguments()
                        .get("path")
                        .and_then(Value::as_str)
                        .map_or_else(
                            || "the target".to_owned(),
                            |path| display_relative(ctx, path),
                        );
                    return Err(RuntimeError::new(ErrorKind::Tool, defect.message(&display)));
                }
                Ok(Some(version)) => {
                    let mut arguments = prepared.arguments().clone();
                    arguments
                        .as_object_mut()
                        .ok_or_else(|| RuntimeError::tool("prepared edit arguments are invalid"))?
                        .insert(
                            crate::edit::EXPECTED_VERSION_FIELD.to_owned(),
                            serde_json::to_value(version).map_err(|_| {
                                RuntimeError::internal("file version could not be encoded")
                            })?,
                        );
                    return Ok(PreparedToolCall::new(
                        prepared.call_id().clone(),
                        prepared.tool(),
                        arguments,
                        prepared.required_permissions().clone(),
                        prepared.resource().clone(),
                        prepared.effects().clone(),
                        prepared.display().clone(),
                    ));
                }
                Ok(None) => {}
            }
        }
        Ok(prepared)
    }

    async fn invoke(
        &self,
        prepared: PreparedToolCall,
        ctx: &InvocationContext,
    ) -> Result<ToolOutcome, RuntimeError> {
        let tool = prepared.tool().to_owned();
        let call_id = prepared.call_id().as_str().to_owned();
        let mutates = prepared.effects().mutates();
        let target = |wanted: &str| {
            (tool == wanted)
                .then(|| {
                    prepared
                        .arguments()
                        .get("path")
                        .and_then(Value::as_str)
                        .and_then(|path| crate::support::resolve(ctx, path).ok())
                })
                .flatten()
        };
        let edit_path = target("edit");
        let read_path = target("read");
        let before = match &edit_path {
            Some(path) => bounded_capability_image(ctx, path),
            None => Ok(None),
        };
        let outcome = self.inner.invoke(prepared, ctx).await;
        if tool == "read" {
            self.observe_read(&outcome, &read_path);
        }
        match (&outcome, edit_path, before) {
            (Ok(outcome), Some(path), Ok(before)) if !outcome.is_error => {
                // `bounded_image` returns `None` for a path that is not there,
                // which after a successful call is exactly what a completed
                // delete looks like. Both images absent is the ambiguous case:
                // nothing existed before and nothing exists now.
                match (bounded_capability_image(ctx, &path), before.is_some()) {
                    (Ok(Some(after)), _) => self.record(ToolMutation::Exact(EditMutation {
                        call_id: call_id.clone(),
                        path,
                        before_hash: hash(before.as_deref()),
                        after_hash: hash(Some(&after)),
                        before,
                        after: Some(after),
                        recovery_path: None,
                    })),
                    (Ok(None), true) => self.record(ToolMutation::Exact(EditMutation {
                        call_id: call_id.clone(),
                        path,
                        before_hash: hash(before.as_deref()),
                        after_hash: hash(None),
                        before,
                        after: None,
                        recovery_path: None,
                    })),
                    _ => self.record(ToolMutation::Ambiguous {
                        call_id: call_id.clone(),
                        tool: tool.clone(),
                    }),
                }
            }
            (_, Some(_), Err(_)) if tool == "edit" => {
                self.record(ToolMutation::Ambiguous {
                    call_id: call_id.clone(),
                    tool: tool.clone(),
                });
            }
            (_, _, _) if mutates && tool != "edit" => {
                self.record(ToolMutation::Ambiguous { call_id, tool });
            }
            _ => {}
        }
        outcome
    }
}

impl ObservedTool {
    /// Records a completed read, and whether it showed the whole file.
    ///
    /// "Whole file" is derived from the outcome the read actually produced
    /// rather than from the requested arguments: a `limit` larger than the file
    /// is a full view, and a caller that omits both is only a full view because
    /// the default happened to cover it.
    fn observe_read(&self, outcome: &Result<ToolOutcome, RuntimeError>, path: &Option<PathBuf>) {
        let (Ok(outcome), Some(path)) = (outcome, path) else {
            return;
        };
        if outcome.is_error {
            return;
        }
        let total = outcome.value.get("lines").and_then(Value::as_u64);
        let shown = outcome
            .value
            .get("shown")
            .and_then(Value::as_array)
            .and_then(|range| match range.as_slice() {
                [first, last] => Some((first.as_u64()?, last.as_u64()?)),
                _ => None,
            });
        let full = matches!((total, shown), (Some(total), Some((1, last))) if last == total);
        let Some(version) = outcome
            .value
            .get("_smith_file_version")
            .cloned()
            .and_then(|value| serde_json::from_value(value).ok())
        else {
            return;
        };
        self.reads.record(path.clone(), full, version);
    }
}

/// Renders a prepared canonical path the way the tools themselves would.
fn display_relative(ctx: &PreparationContext, canonical: &str) -> String {
    std::path::Path::new(canonical)
        .strip_prefix(ctx.workspace.root())
        .map_or_else(|_| canonical.to_owned(), |path| path.display().to_string())
}
