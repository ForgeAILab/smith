use super::*;

const MAX_COMMAND_ARGUMENTS: usize = 256;
const MAX_COMMAND_ARGUMENT_BYTES: usize = 64 * 1024;
const MAX_COMMAND_ENVIRONMENT_ENTRIES: usize = 256;
const MAX_COMMAND_ENVIRONMENT_BYTES: usize = 1024 * 1024;

pub(super) fn resolve_command_provider(
    provenance: &Provenance,
    scope: &str,
) -> Result<Option<ResolvedCommandProvider>, ConfigError> {
    let executable_key = format!("{scope}.command.executable");
    let Some(executable) = text(provenance, &executable_key)? else {
        return Ok(None);
    };
    require_user_process_source(&executable.source)?;
    if executable.value.is_empty() || executable.value.contains('\0') {
        return Err(ConfigError::InvalidValue {
            source: executable.source,
            message: "a command provider executable must be a non-empty path without NUL"
                .to_owned(),
        });
    }
    let executable_path = PathBuf::from(&executable.value);
    if !executable_path.is_absolute() {
        return Err(ConfigError::InvalidValue {
            source: executable.source,
            message:
                "a command provider executable must be an absolute path; Smith does not search PATH"
                    .to_owned(),
        });
    }
    let executable = Sourced::new(executable_path, executable.source);

    let args = list(provenance, &format!("{scope}.command.args"))?;
    if let Some(args) = &args {
        require_user_process_source(&args.source)?;
        if args.value.len() > MAX_COMMAND_ARGUMENTS {
            return Err(ConfigError::InvalidValue {
                source: args.source.clone(),
                message: format!(
                    "a command provider accepts at most {MAX_COMMAND_ARGUMENTS} fixed arguments"
                ),
            });
        }
        if args
            .value
            .iter()
            .any(|argument| argument.len() > MAX_COMMAND_ARGUMENT_BYTES || argument.contains('\0'))
        {
            return Err(ConfigError::InvalidValue {
                source: args.source.clone(),
                message: format!(
                    "each command argument must be at most {MAX_COMMAND_ARGUMENT_BYTES} bytes and contain no NUL"
                ),
            });
        }
    }

    let cwd = text(provenance, &format!("{scope}.command.cwd"))?
        .map(|cwd| {
            require_user_process_source(&cwd.source)?;
            let value = if cwd.value == "workspace" {
                CommandWorkingDirectory::Workspace
            } else {
                if cwd.value.is_empty() || cwd.value.contains('\0') {
                    return Err(ConfigError::InvalidValue {
                        source: cwd.source,
                        message: "a command provider cwd must be `workspace` or an absolute path without NUL"
                            .to_owned(),
                    });
                }
                let path = PathBuf::from(&cwd.value);
                if !path.is_absolute() {
                    return Err(ConfigError::InvalidValue {
                        source: cwd.source,
                        message: "a command provider cwd must be exactly `workspace` or an absolute path"
                            .to_owned(),
                    });
                }
                CommandWorkingDirectory::Absolute(path)
            };
            Ok(Sourced::new(value, cwd.source))
        })
        .transpose()?;

    let env_prefix = format!("{scope}.command.env.");
    let env_keys: Vec<String> = provenance
        .keys()
        .filter(|key| key.starts_with(&env_prefix))
        .map(str::to_owned)
        .collect();
    if env_keys.len() > MAX_COMMAND_ENVIRONMENT_ENTRIES {
        let source = provenance
            .winner(&env_keys[0])
            .expect("a discovered environment key has a winner")
            .source
            .clone();
        return Err(ConfigError::InvalidValue {
            source,
            message: format!(
                "a command provider accepts at most {MAX_COMMAND_ENVIRONMENT_ENTRIES} environment entries"
            ),
        });
    }
    let mut env = BTreeMap::new();
    let mut environment_bytes = 0usize;
    for key in env_keys {
        let name = unquote_segment(&key[env_prefix.len()..]);
        let entry = provenance
            .winner(&key)
            .expect("a discovered environment key has a winner");
        require_user_process_source(&entry.source)?;
        if name.is_empty() || name.contains(['=', '\0']) {
            return Err(ConfigError::InvalidValue {
                source: entry.source.clone(),
                message:
                    "a command environment name must be non-empty and contain neither `=` nor NUL"
                        .to_owned(),
            });
        }
        let value = match &entry.value {
            SettingValue::Text(reference) => {
                let sourced = Sourced::new(reference.clone(), entry.source.clone());
                validate_credential(&sourced)?;
                McpValue::Credential(reference.clone())
            }
            SettingValue::Secret(literal) => McpValue::Literal(literal.clone()),
            other => return Err(wrong_kind(entry, other, "a string")),
        };
        let value_len = match &value {
            McpValue::Credential(reference) => reference.len(),
            McpValue::Literal(literal) => literal.expose().len(),
        };
        if match &value {
            McpValue::Credential(reference) => reference.contains('\0'),
            McpValue::Literal(literal) => literal.expose().contains('\0'),
        } {
            return Err(ConfigError::InvalidValue {
                source: entry.source.clone(),
                message: "a command environment value cannot contain NUL".to_owned(),
            });
        }
        environment_bytes = environment_bytes
            .saturating_add(name.len())
            .saturating_add(value_len);
        if environment_bytes > MAX_COMMAND_ENVIRONMENT_BYTES {
            return Err(ConfigError::InvalidValue {
                source: entry.source.clone(),
                message: format!(
                    "a command provider environment may contain at most {MAX_COMMAND_ENVIRONMENT_BYTES} bytes"
                ),
            });
        }
        env.insert(name, Sourced::new(value, entry.source.clone()));
    }

    Ok(Some(ResolvedCommandProvider {
        executable,
        args,
        cwd,
        env,
    }))
}

pub(super) fn require_user_process_source(source: &Source) -> Result<(), ConfigError> {
    if source.layer == Layer::UserFile {
        return Ok(());
    }
    Err(ConfigError::InvalidValue {
        source: source.clone(),
        message: "command-provider process settings are user-scoped; project configuration may select an existing provider but cannot define or override its process"
            .to_owned(),
    })
}
