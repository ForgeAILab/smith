//! Command-provider configuration, provenance, authority, and redaction.

use std::path::Path;

use smith_config::model::{
    CommandProviderSection, ConfigFile, KIND_COMMAND_JSONL, ProviderSection,
};
use smith_config::resolve::{
    CommandWorkingDirectory, ConfigError, Layer, McpValue, ResolveRequest, SettingValue, resolve,
};
use tempfile::TempDir;

struct Fixture {
    home: TempDir,
    project: TempDir,
}

impl Fixture {
    fn new() -> Self {
        let fixture = Self {
            home: tempfile::tempdir().expect("a home root"),
            project: tempfile::tempdir().expect("a project root"),
        };
        std::fs::create_dir_all(fixture.home.path().join(".smith")).expect("a user config dir");
        std::fs::create_dir_all(fixture.project.path().join(".smith"))
            .expect("a project config dir");
        fixture
    }

    fn write_user(&self, text: &str) {
        std::fs::write(self.home.path().join(".smith/config.toml"), text).expect("a user config");
    }

    #[cfg(unix)]
    fn write_private_user(&self, text: &str) {
        use std::os::unix::fs::PermissionsExt;

        let path = self.home.path().join(".smith/config.toml");
        std::fs::write(&path, text).expect("a user config");
        std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o600))
            .expect("owner-only user config");
    }

    fn write_project(&self, text: &str) {
        std::fs::write(self.project.path().join(".smith/config.toml"), text)
            .expect("a project config");
    }

    fn request(&self) -> ResolveRequest {
        ResolveRequest::new(self.project.path()).with_home_dir(self.home.path())
    }
}

fn selection() -> &'static str {
    r#"
default_profile = "local"

[profiles.local]
provider = "bridge"
model = "local-model"

[models."bridge/local-model"]
context_tokens = 32768
max_input_tokens = 28672
max_output_tokens = 4096
"#
}

fn command_provider(executable: &Path) -> String {
    format!(
        r#"
[providers.bridge]
kind = "command-jsonl"

[providers.bridge.command]
executable = {executable:?}
args = ["serve-smith"]
cwd = "workspace"

[providers.bridge.command.env]
BRIDGE_TOKEN = "env:MODEL_BRIDGE_TOKEN"
"#,
        executable = executable.display().to_string()
    )
}

#[test]
fn command_section_is_strict_and_round_trips() {
    let text = r#"
[providers.bridge]
kind = "command-jsonl"

[providers.bridge.command]
executable = "/opt/smith/model-bridge"
args = ["serve-smith"]
cwd = "workspace"
env = { TOKEN = "env:BRIDGE_TOKEN" }
"#;
    let parsed = ConfigFile::parse(text).expect("a strict command declaration");
    let command = parsed.providers["bridge"]
        .command
        .as_ref()
        .expect("the command table");
    assert_eq!(command.executable, "/opt/smith/model-bridge");
    assert_eq!(command.args, ["serve-smith"]);
    assert_eq!(command.cwd.as_deref(), Some("workspace"));
    assert_eq!(command.env["TOKEN"], "env:BRIDGE_TOKEN");

    let encoded = toml::to_string(&parsed).expect("a serializable command declaration");
    assert_eq!(ConfigFile::parse(&encoded).expect("the round trip"), parsed);

    let unknown = text.replace("cwd = \"workspace\"", "working_dir = \"workspace\"");
    let error = ConfigFile::parse(&unknown).expect_err("an unknown process field");
    assert!(error.message().contains("unknown field"), "{error}");
    assert!(error.message().contains("cwd"), "{error}");

    let missing = ConfigFile::parse(
        "[providers.bridge]\nkind = \"command-jsonl\"\n[providers.bridge.command]\nargs = []\n",
    )
    .expect_err("the executable is required by the file model");
    assert!(missing.message().contains("executable"), "{missing}");
}

#[test]
fn project_profile_can_select_a_user_declared_command_provider() {
    let fixture = Fixture::new();
    let executable = fixture.home.path().join("bin/model-bridge");
    fixture.write_user(&command_provider(&executable));
    fixture.write_project(selection());

    let resolution = resolve(&fixture.request()).expect("a project-selected user provider");
    assert_eq!(resolution.config.provider.kind.value, KIND_COMMAND_JSONL);
    assert_eq!(
        resolution.config.provider.kind.source.layer,
        Layer::UserFile
    );
    assert_eq!(resolution.config.provider.name.source.layer, Layer::Profile);

    let command = resolution
        .config
        .provider
        .command
        .as_ref()
        .expect("resolved process settings");
    assert_eq!(command.executable.value, executable);
    assert_eq!(command.executable.source.layer, Layer::UserFile);
    assert_eq!(
        command.args.as_ref().expect("fixed args").value,
        ["serve-smith"]
    );
    assert!(matches!(
        command.cwd.as_ref().expect("an explicit cwd").value,
        CommandWorkingDirectory::Workspace
    ));
    assert_eq!(
        command.env["BRIDGE_TOKEN"].value.credential(),
        Some("env:MODEL_BRIDGE_TOKEN")
    );

    let explanation = resolution
        .provenance
        .explain("providers.bridge.command.executable")
        .expect("executable provenance");
    assert_eq!(explanation.source.layer, Layer::UserFile);
    assert_eq!(
        explanation.value.to_string(),
        executable.display().to_string()
    );
}

#[cfg(unix)]
#[test]
fn command_environment_literals_are_owner_only_and_redacted() {
    const SECRET: &str = "bridge-token-must-never-render";
    let fixture = Fixture::new();
    let executable = fixture.home.path().join("bin/model-bridge");
    fixture.write_private_user(&format!(
        "{}\n[providers.bridge.command.env]\nBRIDGE_TOKEN = {SECRET:?}\n",
        command_provider(&executable).replace(
            "\n[providers.bridge.command.env]\nBRIDGE_TOKEN = \"env:MODEL_BRIDGE_TOKEN\"\n",
            ""
        ),
    ));
    fixture.write_project(selection());

    let resolution = resolve(&fixture.request()).expect("an owner-only literal environment");
    let value = &resolution.config.provider.command.as_ref().unwrap().env["BRIDGE_TOKEN"].value;
    assert!(matches!(value, McpValue::Literal(_)));
    let explanation = resolution
        .provenance
        .explain("providers.bridge.command.env.BRIDGE_TOKEN")
        .expect("environment provenance");
    assert!(matches!(explanation.value, SettingValue::Secret(_)));
    assert_eq!(explanation.value.to_string(), "[redacted]");
    assert!(!format!("{resolution:?}").contains(SECRET));

    let permissive = Fixture::new();
    permissive.write_user(&format!(
        "{}\n[providers.bridge.command.env]\nBRIDGE_TOKEN = {SECRET:?}\n",
        command_provider(&executable).replace(
            "\n[providers.bridge.command.env]\nBRIDGE_TOKEN = \"env:MODEL_BRIDGE_TOKEN\"\n",
            ""
        ),
    ));
    permissive.write_project(selection());
    let error = resolve(&permissive.request()).expect_err("a permissive secret-bearing file");
    assert!(matches!(error, ConfigError::PlaintextSecret { .. }));
    assert!(!format!("{error:?}").contains(SECRET));
}

#[test]
fn project_files_cannot_define_or_override_command_process_authority() {
    for project_provider in [
        r#"
[providers.bridge]
kind = "command-jsonl"
"#,
        r#"
[providers.bridge.command]
executable = "/tmp/project-controlled-bridge"
"#,
    ] {
        let fixture = Fixture::new();
        let executable = fixture.home.path().join("bin/model-bridge");
        fixture.write_user(&command_provider(&executable));
        fixture.write_project(&format!("{}\n{project_provider}", selection()));

        let error = resolve(&fixture.request()).expect_err("project process authority");
        assert!(matches!(error, ConfigError::InvalidValue { .. }));
        let diagnostic = error.to_string();
        assert!(diagnostic.contains("user-scoped"), "{diagnostic}");
        assert!(!diagnostic.contains("project-controlled-bridge"));
    }
}

#[test]
fn command_option_matrix_is_fail_closed() {
    let fixture = Fixture::new();
    let executable = fixture.home.path().join("bin/model-bridge");
    let base = command_provider(&executable);
    fixture.write_project(selection());

    for option in [
        "base_url = \"https://example.invalid/v1\"",
        "credential = \"env:BRIDGE_TOKEN\"",
        "credentials = [\"env:A\", \"env:B\"]",
        "rotate_at_percent = 90",
        "api_key = \"secret\"",
        "headers = { X_TEST = \"value\" }",
        "response = { reasoning_only = \"text\" }",
    ] {
        let declaration = base.replace(
            "kind = \"command-jsonl\"",
            &format!("kind = \"command-jsonl\"\n{option}"),
        );
        if option.starts_with("api_key") {
            #[cfg(unix)]
            fixture.write_private_user(&declaration);
            #[cfg(not(unix))]
            continue;
        } else {
            fixture.write_user(&declaration);
        }
        let error = resolve(&fixture.request()).expect_err("an HTTP-only command option");
        assert!(matches!(error, ConfigError::IncompatibleOption { .. }));
    }

    fixture.write_user(&base.replace("kind = \"command-jsonl\"", "kind = \"fake\""));
    let error = resolve(&fixture.request()).expect_err("a command table on a native adapter");
    assert!(matches!(error, ConfigError::IncompatibleOption { .. }));

    fixture.write_user("[providers.bridge]\nkind = \"command-jsonl\"\n");
    let error = resolve(&fixture.request()).expect_err("a missing command table");
    assert!(matches!(error, ConfigError::MissingSetting { .. }));
}

#[test]
fn malformed_or_oversized_process_settings_fail_during_resolution() {
    let fixture = Fixture::new();
    fixture.write_project(selection());

    let declarations = [
        r#"
[providers.bridge]
kind = "command-jsonl"
[providers.bridge.command]
executable = "model-bridge"
"#
        .to_owned(),
        r#"
[providers.bridge]
kind = "command-jsonl"
[providers.bridge.command]
executable = "/opt/model-bridge"
cwd = "relative"
"#
        .to_owned(),
        r#"
[providers.bridge]
kind = "command-jsonl"
[providers.bridge.command]
executable = "/opt/model-bridge"
[providers.bridge.command.env]
"BAD=NAME" = "env:TOKEN"
"#
        .to_owned(),
        format!(
            "[providers.bridge]\nkind = \"command-jsonl\"\n[providers.bridge.command]\nexecutable = \"/opt/model-bridge\"\nargs = [{:?}]\n",
            "x".repeat(64 * 1024 + 1)
        ),
    ];

    for declaration in declarations {
        fixture.write_user(&declaration);
        let error = resolve(&fixture.request()).expect_err("invalid static process settings");
        assert!(matches!(error, ConfigError::InvalidValue { .. }));
    }
}

#[test]
fn file_model_construction_keeps_command_values_typed() {
    let file = ConfigFile {
        providers: [(
            "bridge".to_owned(),
            ProviderSection {
                kind: Some(KIND_COMMAND_JSONL.to_owned()),
                command: Some(CommandProviderSection {
                    executable: "/opt/model-bridge".to_owned(),
                    args: vec!["serve-smith".to_owned()],
                    cwd: Some("workspace".to_owned()),
                    env: [("TOKEN".to_owned(), "env:BRIDGE_TOKEN".to_owned())]
                        .into_iter()
                        .collect(),
                }),
                ..ProviderSection::default()
            },
        )]
        .into_iter()
        .collect(),
        ..ConfigFile::default()
    };
    let encoded = toml::to_string(&file).expect("a serializable typed declaration");
    assert!(encoded.contains("command"));
    assert_eq!(ConfigFile::parse(&encoded).unwrap(), file);
}

#[test]
fn explicit_absolute_cwd_is_preserved_as_a_path() {
    let fixture = Fixture::new();
    let executable = fixture.home.path().join("bin/model-bridge");
    let cwd = fixture.project.path().join("bridge-work");
    fixture.write_user(&command_provider(&executable).replace(
        "cwd = \"workspace\"",
        &format!("cwd = {:?}", cwd.display().to_string()),
    ));
    fixture.write_project(selection());

    let resolution = resolve(&fixture.request()).expect("an absolute command cwd");
    assert_eq!(
        resolution
            .config
            .provider
            .command
            .unwrap()
            .cwd
            .unwrap()
            .value,
        CommandWorkingDirectory::Absolute(cwd)
    );
}
