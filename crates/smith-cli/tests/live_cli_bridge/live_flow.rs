//! Token-spending installed-CLI flow helpers.

use super::*;

const CODEX_DEFAULT: &str = "live-bridge-use-cli-default";
const FLOW_TIMEOUT: Duration = Duration::from_secs(900);

fn shell_string(value: &OsStr) -> String {
    // Paths are data, never interpolated as executable shell syntax.
    format!(
        "'{}'",
        value
            .to_str()
            .expect("UTF-8 CLI path")
            .replace('\'', "'\"'\"'")
    )
}

struct LivePath {
    // Drop before the argv recordings' directory is removed, after capture's
    // Process guard has stopped Smith on failure.
    cleanup: BridgeCleanup,
    _directory: tempfile::TempDir,
    path: OsString,
    launches: PathBuf,
}

impl LivePath {
    fn new(cli: &Installed, default_codex: bool) -> Self {
        let directory = tempfile::tempdir().expect("live CLI PATH shim directory");
        let argv_directory = directory.path().join("argv");
        fs::create_dir(&argv_directory).expect("live CLI argv directory");
        let launches = directory.path().join("launches");
        let shim = directory.path().join(cli.kind.program());
        // Smith currently requires cli/<kind>/<model> and always emits -m.
        // A transparent exec shim removes ONLY our sentinel's -m pair for
        // the requested Codex default. Every bridge flag and key passes intact.
        // The launch marker detects owner-pinned executables bypassing PATH.
        let strip = if default_codex {
            format!(
                r#"
remaining=$#
while [ "$remaining" -gt 0 ]; do
    argument=$1
    shift
    remaining=$((remaining - 1))
    if [ "$argument" = '-m' ] && [ "$remaining" -gt 0 ] && [ "$1" = '{CODEX_DEFAULT}' ]; then
        shift
        remaining=$((remaining - 1))
    else
        set -- "$@" "$argument"
    fi
done
"#
            )
        } else {
            String::new()
        };
        let script = format!(
            "#!/bin/sh\nrecord={}/$$\nprintf '%s\\0' \"$@\" > \"$record.tmp\"\n/bin/mv \"$record.tmp\" \"$record.argv\"\n{strip}\nprintf 'launch\\n' >> {}\nexec {} \"$@\"\n",
            shell_string(argv_directory.as_os_str()),
            shell_string(launches.as_os_str()),
            shell_string(cli.binary.as_os_str())
        );
        fs::write(&shim, script).expect("live CLI exec shim");
        fs::set_permissions(&shim, fs::Permissions::from_mode(0o700)).expect("executable shim");
        let mut paths = vec![directory.path().to_owned()];
        paths.extend(std::env::split_paths(
            &std::env::var_os("PATH").unwrap_or_default(),
        ));
        let path = std::env::join_paths(paths).expect("live CLI PATH");
        Self {
            cleanup: BridgeCleanup::from_argv_directory(argv_directory),
            _directory: directory,
            path,
            launches,
        }
    }

    fn assert_launches(&self, count: usize, context: &str) {
        let launches = fs::read_to_string(&self.launches).unwrap_or_default();
        assert_eq!(
            launches.lines().count(),
            count,
            "{context}: Smith bypassed the requested binary/default-model PATH shim; \
             an owner-pinned [harness.<kind>].executable must be unset for this live flow; \
             the test preserves the owner's configuration rather than rewriting it"
        );
    }
}

#[allow(clippy::too_many_arguments)]
fn smith_turn(
    cli: &Installed,
    path: &LivePath,
    project: &Path,
    model: &str,
    approval: &str,
    prompt: &str,
    resume: Option<&str>,
    deadline: Instant,
) -> Vec<Value> {
    let mut command = Command::new(env!("CARGO_BIN_EXE_smith"));
    clean_environment(&mut command);
    command
        .current_dir(project)
        .env("PATH", &path.path)
        .arg("--project")
        .arg(project)
        .args([
            "--model",
            &format!("cli/{}/{model}", cli.kind.kind()),
            "--approval",
            approval,
            "--output-format",
            "stream-json",
            "-p",
            prompt,
        ]);
    if let Some(session) = resume {
        command.args(["--resume", session]);
    }
    let output = capture(&mut command, None, deadline, &cli.context);
    path.cleanup.capture();
    require_success(&output, &cli.context, "headless Smith bridge turn");
    let rows = json_lines(&output.stdout, &cli.context);
    let results: Vec<_> = rows.iter().filter(|row| row["type"] == "result").collect();
    assert_eq!(
        results.len(),
        1,
        "{}: expected one Smith result: {rows:?}",
        cli.context
    );
    assert_eq!(
        results[0]["status"], "ok",
        "{}: Smith result.status: {}",
        cli.context, results[0]
    );
    assert_eq!(
        results[0]["error"],
        Value::Null,
        "{}: Smith result.error",
        cli.context
    );
    rows
}

fn payloads(rows: &[Value]) -> impl Iterator<Item = &Value> {
    rows.iter()
        .filter(|row| row["type"] == "runtime_event")
        .map(|row| &row["event"]["payload"])
}

fn agent_call(rows: &[Value], context: &str) {
    let payloads: Vec<_> = payloads(rows).collect();
    let requests: Vec<_> = payloads
        .iter()
        .enumerate()
        .filter(|(_, payload)| {
            payload["event"] == "tool_call_requested" && payload["name"] == "agent"
        })
        .collect();
    assert!(
        !requests.is_empty(),
        "{context}: no ordinary agent tool_call_requested; \
        owner profile must enable delegation and authorize agent: {rows:?}"
    );
    for (index, request) in requests {
        let call = request["call"]
            .as_str()
            .unwrap_or_else(|| panic!("{context}: tool_call_requested.call missing: {request}"));
        let completions: Vec<_> = payloads
            .iter()
            .enumerate()
            .filter(|(_, payload)| {
                payload["event"] == "tool_call_completed"
                    && payload["name"] == "agent"
                    && payload["call"] == call
            })
            .collect();
        assert_eq!(
            completions.len(),
            1,
            "{context}: agent call {call} must complete once: {rows:?}"
        );
        assert!(
            index < completions[0].0,
            "{context}: agent completion preceded request"
        );
        assert_eq!(
            completions[0].1["is_error"], false,
            "{context}: agent call {call} failed: {rows:?}"
        );
    }
    assert!(
        payloads
            .iter()
            .all(|payload| !(payload["name"] == "mcp__smith__agent"
                && payload["event"]
                    .as_str()
                    .is_some_and(|name| name.starts_with("external_tool")))),
        "{context}: CLI echo mcp__smith__agent was reported as an external tool: {rows:?}"
    );
    let result = event(rows, "result", context);
    assert!(
        result["output"]
            .as_str()
            .is_some_and(|text| text.contains("children")),
        "{context}: successful Smith result.output did not report agent list's children: {result}"
    );
}

fn identity(rows: &[Value], context: &str) -> (String, String) {
    let result = event(rows, "result", context);
    let smith = result["session_id"]
        .as_str()
        .filter(|text| !text.is_empty())
        .unwrap_or_else(|| panic!("{context}: result.session_id missing: {result}"));
    let sessions: Vec<_> = payloads(rows)
        .filter(|payload| payload["event"] == "external_session_started")
        .collect();
    assert_eq!(
        sessions.len(),
        1,
        "{context}: expected one external_session_started: {rows:?}"
    );
    let external = sessions[0]["session"]
        .as_str()
        .filter(|text| !text.is_empty())
        .unwrap_or_else(|| {
            panic!(
                "{context}: external_session_started.session missing: {}",
                sessions[0]
            )
        });
    (smith.to_owned(), external.to_owned())
}

fn live_request(project: &Path, model: &str) -> smith_config::resolve::ResolveRequest {
    use smith_config::resolve::{KnownModule, Overrides, ResolveRequest};

    // The same first-party module ids accepted by the CLI's build catalog.
    let modules = [
        ("web", cfg!(feature = "module-web")),
        ("vision", cfg!(feature = "module-vision")),
        (
            "image-generation",
            cfg!(feature = "module-image-generation"),
        ),
        ("budget-notice", cfg!(feature = "module-budget-notice")),
    ]
    .into_iter()
    .map(|(id, compiled_in)| KnownModule {
        id: id.into(),
        default_enabled: true,
        compiled_in,
        legacy_enabled_key: (id == "image-generation")
            .then(|| "tools.image_generation.enabled".into()),
    })
    .collect();
    ResolveRequest::new(project)
        .with_known_modules(modules)
        .with_env(std::env::vars())
        .with_cli(Overrides {
            model: Some(model.to_owned()),
            ..Overrides::default()
        })
}

fn saved_live_session(
    paths: &smith_runtime::session::SessionPaths,
    session: &str,
    context: &str,
) -> (Value, Vec<Value>) {
    let snapshot_path = paths.directory().join(format!("{session}.snapshot.json"));
    let snapshot = serde_json::from_slice(&fs::read(&snapshot_path).unwrap_or_else(|error| {
        panic!(
            "{context}: saved session {}: {error}",
            snapshot_path.display()
        )
    }))
    .unwrap_or_else(|error| panic!("{context}: saved snapshot JSON: {error}"));
    let journal_path = paths.directory().join(format!("{session}.jsonl"));
    let journal = fs::read(&journal_path).unwrap_or_else(|error| {
        panic!(
            "{context}: saved journal {}: {error}",
            journal_path.display()
        )
    });
    let events = json_lines(&journal, context)
        .into_iter()
        .filter(|row| row["record"] == "event")
        .map(|row| row["event"].clone())
        .collect();
    (snapshot, events)
}

// Check committed calls/results as well as the canonical journal, so prose
// claiming a tool ran (or a CLI-owned duplicate) cannot satisfy a live check.
fn history_tool<'a>(
    snapshot: &'a Value,
    journal: &[Value],
    name: &str,
    action: Option<&str>,
    context: &str,
) -> (usize, &'a Value, &'a Value) {
    let parts: Vec<_> = snapshot["snapshot"]["history"]
        .as_array()
        .unwrap_or_else(|| panic!("{context}: saved history missing: {snapshot}"))
        .iter()
        .flat_map(|message| message["content"].as_array().expect("history content"))
        .collect();
    let calls: Vec<_> = parts
        .iter()
        .enumerate()
        .filter(|(_, part)| {
            part["type"] == "tool_call"
                && part["name"] == name
                && action.is_none_or(|action| part["arguments"]["action"] == action)
        })
        .collect();
    assert_eq!(
        calls.len(),
        1,
        "{context}: expected one committed {name} {action:?} call; model history: {parts:?}"
    );
    let (index, call) = calls[0];
    assert!(call["id"].is_string(), "{context}: call id missing: {call}");
    let results: Vec<_> = parts
        .iter()
        .enumerate()
        .filter(|(_, part)| part["type"] == "tool_result" && part["call_id"] == call["id"])
        .collect();
    assert_eq!(
        results.len(),
        1,
        "{context}: {name} result missing/duplicated: {parts:?}"
    );
    let (result_index, result) = results[0];
    assert!(
        index < result_index,
        "{context}: {name} result precedes call: {parts:?}"
    );
    assert_eq!(result["name"], name, "{context}: {result}");
    assert_eq!(
        result["is_error"], false,
        "{context}: {name} failed: {result}"
    );
    let recorded = |event: &str| {
        journal
            .iter()
            .enumerate()
            .filter(|(_, row)| {
                row["payload"]["event"] == event && row["payload"]["call"] == call["id"]
            })
            .collect::<Vec<_>>()
    };
    let requests = recorded("tool_call_requested");
    let completions = recorded("tool_call_completed");
    assert_eq!(
        requests.len(),
        1,
        "{context}: {name} ordinary request: {journal:?}"
    );
    assert_eq!(
        completions.len(),
        1,
        "{context}: {name} ordinary completion: {journal:?}"
    );
    assert!(
        requests[0].0 < completions[0].0,
        "{context}: {name} journal order: {journal:?}"
    );
    assert_eq!(
        requests[0].1["payload"]["name"], name,
        "{context}: {journal:?}"
    );
    assert_eq!(
        completions[0].1["payload"]["name"], name,
        "{context}: {journal:?}"
    );
    assert_eq!(
        completions[0].1["payload"]["is_error"], false,
        "{context}: {journal:?}"
    );
    let alias = name.replace('.', "_");
    assert!(
        journal.iter().all(|row| {
            let payload = &row["payload"];
            let observed = payload["name"].as_str().unwrap_or("");
            !(payload["event"]
                .as_str()
                .is_some_and(|event| event.starts_with("external_tool"))
                && (observed == name
                    || observed == alias
                    || observed == format!("mcp__smith__{name}")
                    || observed == format!("mcp__smith__{alias}")))
        }),
        "{context}: {name} recorded as a CLI-owned duplicate: {journal:?}"
    );
    (index, call, result)
}

fn history_result_text(result: &Value) -> String {
    result["content"]
        .as_array()
        .expect("tool result content")
        .iter()
        .filter_map(|part| part["text"].as_str())
        .collect::<Vec<_>>()
        .join("\n")
}

fn history_result_json(result: &Value, context: &str) -> Value {
    let text = history_result_text(result);
    serde_json::from_str(&text)
        .unwrap_or_else(|error| panic!("{context}: expected tool result JSON: {error}: {result}"))
}

pub(super) fn live_flow(kind: Kind) {
    // No live remote-MCP-behind-approval step: headless cannot answer an
    // approval interactively. bridge_approvals:: covers the real endpoint and relay.
    let cli = Installed::discover(kind);
    let deadline = Instant::now() + FLOW_TIMEOUT;
    let model = match kind {
        Kind::Claude => std::env::var("LIVE_CLAUDE_MODEL").unwrap_or_else(|_| "haiku".into()),
        Kind::Codex => std::env::var("LIVE_CODEX_MODEL").unwrap_or_else(|_| CODEX_DEFAULT.into()),
    };
    assert!(
        !model.trim().is_empty(),
        "{}: live model override must not be empty",
        cli.context
    );
    let path = LivePath::new(&cli, kind == Kind::Codex && model == CODEX_DEFAULT);
    let project = tempfile::tempdir().expect("temporary live Smith project");
    let child_model = std::env::var_os("LIVE_CHILD_MODEL")
        .map(|value| value.into_string().expect("LIVE_CHILD_MODEL must be UTF-8"));
    let child_profile = child_model.as_ref().map(|model| {
        assert!(!model.trim().is_empty(), "LIVE_CHILD_MODEL must not be empty");
        let child = smith_config::resolve::resolve(&live_request(project.path(), model))
            .unwrap_or_else(|error| panic!("{}: LIVE_CHILD_MODEL={model}: {error}", cli.context));
        assert!(child.config.harness.is_none(),
            "{}: LIVE_CHILD_MODEL={model} must resolve to a provider child, not a CLI harness",
            cli.context);
        // agent.spawn accepts profile, not model. Bind the requested model in
        // this disposable project only, retaining the owner's providers/logins.
        let profile = format!("live-child-{}", &uuid::Uuid::new_v4().simple().to_string()[..12]);
        let model = &child.config.model.value;
        let model = model
            .strip_prefix(&format!("{}/", child.config.provider.name.value))
            .unwrap_or(model);
        fs::create_dir(project.path().join(".smith")).expect("temporary project config directory");
        fs::write(project.path().join(".smith/config.toml"), format!(
            "[profiles.{profile}]\nprovider = {}\nmodel = {}\nposture = \"review\"\nuse = [\"child\"]\n",
            json!(child.config.provider.name.value), json!(model)
        )).expect("temporary child-only profile");
        profile
    });
    if child_model.is_none() {
        println!(
            "{}: child check skipped; enable with LIVE_CHILD_MODEL=<Smith model id>",
            cli.context
        );
    }
    let advisor_enabled = std::env::var("LIVE_ADVISOR").is_ok_and(|value| value == "1");
    if !advisor_enabled {
        println!(
            "{}: advisor check skipped; enable with LIVE_ADVISOR=1",
            cli.context
        );
    }
    let request = live_request(project.path(), &format!("cli/{}/{model}", kind.kind()));
    let resolution = smith_config::resolve::resolve(&request)
        .unwrap_or_else(|error| panic!("{}: live Smith config: {error}", cli.context));
    if advisor_enabled {
        let advisor = resolution.config.agent.profile.advisor.clone().unwrap_or_else(|| {
            panic!("{}: LIVE_ADVISOR=1 requires an advisor, but the owner's config resolves no advisor for this profile", cli.context)
        });
        smith_config::resolve::resolve(&request.clone().with_advisor_route(advisor))
            .unwrap_or_else(|error| {
                panic!(
                    "{}: LIVE_ADVISOR=1 advisor cannot resolve: {error}",
                    cli.context
                )
            });
    }
    assert!(
        resolution.config.persistence.enabled.value
            && resolution.config.persistence.journal_events.value,
        "{}: live assertions require saved sessions and event journals in the owner's config",
        cli.context
    );
    let session_paths = smith_runtime::host::paths(&resolution.config, project.path())
        .unwrap_or_else(|error| panic!("{}: live session paths: {error}", cli.context));
    // The section's name plus distinctive descriptor prose gives it the top
    // rank; generic "headless protocol" would also retrieve the outline.
    const SKILL_QUERY: &str = "headless input output modes bounded UTF-8";
    const SKILL_NAME: &str = "smith.headless.input-and-output-modes";
    const SKILL_PHRASE: &str = "Prompt stdin is read once, as bounded UTF-8, before the turn.";
    let search_args = json!({"query": SKILL_QUERY, "domain": "skill", "max_results": 1});
    let codeword = format!("bridge-{}", uuid::Uuid::new_v4().simple());
    let first_prompt = format!(
        "Remember codeword {codeword} for the next turn. Use smith MCP tools in order: \
         agent {{\"action\":\"list\"}}; registry.search {search_args}; then registry.activate \
         {{\"ids\":[\"skill:<returned name>\"]}} using the search result's first id. \
         Report agent's JSON including children. No other tools except tool search to discover smith tools."
    );
    let first = smith_turn(
        &cli,
        &path,
        project.path(),
        &model,
        "deny",
        &first_prompt,
        None,
        deadline,
    );
    path.assert_launches(1, &cli.context);
    agent_call(&first, &cli.context);
    let (smith, external) = identity(&first, &cli.context);
    let (snapshot, journal) = saved_live_session(&session_paths, &smith, &cli.context);
    let (search_index, search, search_result) =
        history_tool(&snapshot, &journal, "registry.search", None, &cli.context);
    assert_eq!(
        search["arguments"], search_args,
        "{}: search arguments: {search}",
        cli.context
    );
    let discovery = history_result_json(search_result, &cli.context);
    assert_eq!(
        discovery["cards"][0]["id"],
        json!({"domain":"skill", "name":SKILL_NAME}),
        "{}: expected deterministic built-in top result: {discovery}",
        cli.context
    );
    let (activate_index, activate, activation_result) =
        history_tool(&snapshot, &journal, "registry.activate", None, &cli.context);
    assert!(
        search_index < activate_index,
        "{}: activation must follow search: {snapshot}",
        cli.context
    );
    let search_completed = journal
        .iter()
        .position(|row| {
            row["payload"]["event"] == "tool_call_completed"
                && row["payload"]["call"] == search["id"]
        })
        .expect("search completion");
    let activation_requested = journal
        .iter()
        .position(|row| {
            row["payload"]["event"] == "tool_call_requested"
                && row["payload"]["call"] == activate["id"]
        })
        .expect("activation request");
    assert!(
        search_completed < activation_requested,
        "{}: activation ran before search returned: {journal:?}",
        cli.context
    );
    let skill_id = format!("skill:{SKILL_NAME}");
    assert_eq!(
        activate["arguments"],
        json!({"ids":[skill_id]}),
        "{}: {activate}",
        cli.context
    );
    let activation = history_result_json(activation_result, &cli.context);
    assert!(
        activation["activated"]
            .as_array()
            .is_some_and(|entries| entries.iter().any(|entry| entry["id"] == skill_id
                && entry["instructions"]
                    .as_str()
                    .is_some_and(|text| text.contains(SKILL_PHRASE)))),
        "{}: committed activation lacks built-in instruction text: {activation}; model history: {snapshot}",
        cli.context
    );
    println!(
        "{}: turn one Smith session={smith}, external session={external}",
        cli.context
    );
    // Calling agent again proves the resumed turn got a fresh working bridge;
    // merely recalling a word would also pass if the bridge disappeared.
    let mut second_prompt = String::from(
        "Recall the exact codeword. Use smith MCP agent \
        {\"action\":\"list\"} again and report its JSON including children.",
    );
    if let Some(profile) = &child_profile {
        let spawn_args = json!({"action":"spawn", "task":"Reply with exactly the word PONG.", "profile":profile});
        second_prompt.push_str(&format!(" Then call agent {spawn_args}, then agent \
            {{\"action\":\"wait\",\"child_id\":\"<returned spawned id>\",\"timeout_ms\":60000}} once. Report its result."));
    }
    if advisor_enabled {
        second_prompt.push_str(" Then call smith MCP advisor {} once and report its answer.");
    }
    second_prompt.push_str(" No other tools except tool search to discover smith tools.");
    // A spawn always requires approval, and headless cannot answer one. Using
    // allow-all here widens only Smith-governed bridge calls in this disposable
    // project; the CLI's own tools follow allow_own_tools.
    let second = smith_turn(
        &cli,
        &path,
        project.path(),
        &model,
        if child_profile.is_some() {
            "allow-all"
        } else {
            "deny"
        },
        &second_prompt,
        Some(&smith),
        deadline,
    );
    path.assert_launches(2, &cli.context);
    agent_call(&second, &cli.context);
    let (resumed_smith, resumed_external) = identity(&second, &cli.context);
    assert_eq!(
        resumed_smith, smith,
        "{}: resumed Smith session_id changed",
        cli.context
    );
    assert_eq!(
        resumed_external, external,
        "{}: resumed external session changed",
        cli.context
    );
    assert!(
        event(&second, "result", &cli.context)["output"]
            .as_str()
            .is_some_and(|text| text.contains(&codeword)),
        "{}: resumed answer did not recall codeword {codeword}: {second:?}",
        cli.context
    );
    let (snapshot, journal) = saved_live_session(&session_paths, &smith, &cli.context);
    if let Some(profile) = &child_profile {
        let (spawn_index, spawn, spawn_result) =
            history_tool(&snapshot, &journal, "agent", Some("spawn"), &cli.context);
        assert_eq!(
            spawn["arguments"],
            json!({"action":"spawn", "task":"Reply with exactly the word PONG.", "profile":profile}),
            "{}: requested provider child: {spawn}",
            cli.context
        );
        let spawned = history_result_json(spawn_result, &cli.context);
        let child_id = spawned["spawned"]
            .as_str()
            .filter(|id| !id.is_empty())
            .unwrap_or_else(|| panic!("{}: child was not spawned: {spawned}", cli.context));
        let (wait_index, wait, wait_result) =
            history_tool(&snapshot, &journal, "agent", Some("wait"), &cli.context);
        assert!(
            spawn_index < wait_index,
            "{}: wait precedes spawn: {snapshot}",
            cli.context
        );
        assert_eq!(
            wait["arguments"],
            json!({"action":"wait", "child_id":child_id, "timeout_ms":60000}),
            "{}: child wait must be bounded and address spawned child: {wait}",
            cli.context
        );
        let waited = history_result_json(wait_result, &cli.context);
        assert_eq!(waited["child_id"], child_id, "{}: {waited}", cli.context);
        assert_eq!(
            waited["timed_out"], false,
            "{}: child missed wait deadline: {waited}",
            cli.context
        );
        assert!(
            waited["result"]
                .as_str()
                .is_some_and(|text| text.contains("PONG")),
            "{}: child result lacks PONG: {waited}; model history: {snapshot}",
            cli.context
        );
    }
    if advisor_enabled {
        let (_, _, advisor_result) =
            history_tool(&snapshot, &journal, "advisor", None, &cli.context);
        assert!(
            !history_result_text(advisor_result).trim().is_empty(),
            "{}: advisor result was empty: {advisor_result}",
            cli.context
        );
        assert!(
            journal.iter().any(|row| row["payload"]["event"] == "usage"
                && row["payload"]["record"]["provenance"]["purpose"] == "advisor"),
            "{}: session usage lacks advisor attribution: {journal:?}",
            cli.context
        );
    }
    let endpoints = path.cleanup.endpoints();
    assert_eq!(
        endpoints.len(),
        2,
        "{}: one endpoint per live turn",
        cli.context
    );
    for endpoint in endpoints {
        assert!(
            !endpoint.exists() && !endpoint.parent().unwrap().exists(),
            "{}: live turn left its bridge directory: {}",
            cli.context,
            endpoint.display()
        );
    }
    println!(
        "{}: live bridge + resume passed; Smith session={smith}; external session={external}",
        cli.context
    );
}
