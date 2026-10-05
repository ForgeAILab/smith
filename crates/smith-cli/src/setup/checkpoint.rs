use super::{
    CheckpointKeyProvider, CheckpointSetupContext, ConfigFile, ConfigReadiness, ConfigSecret,
    ConfiguredCheckpointKeyProvider, Context, ExistingCheckpointSource, Generate, Path, PathBuf,
    PersistenceSection, ResolveRequest, Result, SetupOutcome, Write, Zeroizing, canonical_start,
    inspect, io, prepare_checkpoint_key_source_removal, prepare_user_config_edit,
    setup_environment_reference,
};

pub(super) async fn run_checkpoint_key_setup(project: Option<PathBuf>) -> Result<SetupOutcome> {
    let start = canonical_start(project.as_deref())?;
    let request = ResolveRequest::new(&start).with_env(std::env::vars());
    let context = match inspect(&request) {
        ConfigReadiness::Ready(resolution) => CheckpointSetupContext {
            user_dir: resolution.layout.user_dir.clone(),
            sessions_dir: resolution.config.persistence.sessions_dir.value.clone(),
            source: if resolution.config.persistence.checkpoint_key.is_some() {
                ExistingCheckpointSource::Inline
            } else if let Some(reference) = &resolution.config.persistence.checkpoint_key_credential
            {
                ExistingCheckpointSource::Credential(reference.value.clone())
            } else {
                ExistingCheckpointSource::Platform
            },
        },
        ConfigReadiness::Unconfigured(context) => CheckpointSetupContext {
            sessions_dir: context.layout.user_dir.join("sessions"),
            user_dir: context.layout.user_dir,
            source: ExistingCheckpointSource::Platform,
        },
        ConfigReadiness::Invalid(error) => {
            return Err(anyhow::anyhow!(error))
                .context("configuration is invalid; checkpoint-key setup will not overwrite it");
        }
    };

    println!("Smith checkpoint protection");
    println!("  1. Store in config (no future prompts)");
    println!("  2. Use an environment-variable reference");
    println!("  3. Use OS protected storage (may request an unlock password)");
    println!("  q. Cancel");
    let choice = prompt_line("Choice [1]: ")?;
    match choice.trim() {
        "" | "1" => setup_inline_checkpoint_key(&context),
        "2" => setup_checkpoint_environment(&context),
        "3" => setup_platform_checkpoint_key(&context),
        "q" | "Q" => Ok(SetupOutcome::Cancelled),
        _ => anyhow::bail!("choose `1`, `2`, `3`, or `q`"),
    }
}

fn setup_inline_checkpoint_key(context: &CheckpointSetupContext) -> Result<SetupOutcome> {
    println!();
    println!(
        "The key will be plaintext in owner-only ~/.smith/config.toml. Same-user processes and backups can read it."
    );
    println!(
        "Exact checkpoints remain authenticated-encrypted and startup will not query Keychain."
    );
    if prompt_line("Type `yes` to generate and store it: ")?.trim() != "yes" {
        return Ok(SetupOutcome::Cancelled);
    }
    refuse_unsafe_checkpoint_rotation(context, &ExistingCheckpointSource::GeneratedInline)?;

    let bytes = Zeroizing::new(
        <[u8; 32]>::try_generate().context("generating checkpoint protection material")?,
    );
    let encoded = Zeroizing::new(hex_encode(bytes.as_slice()));
    let secret = agent_runtime_core::store::Secret::new(encoded.as_str());
    let provider = ConfiguredCheckpointKeyProvider::new(&secret)
        .map_err(|error| anyhow::anyhow!(error))
        .context("validating checkpoint protection material")?;
    provider
        .load_or_create()
        .map_err(|error| anyhow::anyhow!(error))
        .context("preflighting checkpoint protection material")?;

    let patch = ConfigFile {
        persistence: Some(PersistenceSection {
            checkpoint_key: Some(ConfigSecret::new(encoded.as_str())),
            checkpoint_key_credential: None,
            ..PersistenceSection::default()
        }),
        ..ConfigFile::default()
    };
    commit_checkpoint_patch(&context.user_dir, patch)
}

fn setup_checkpoint_environment(context: &CheckpointSetupContext) -> Result<SetupOutcome> {
    let variable = prompt_line("Environment variable name [SMITH_CHECKPOINT_SECRET]: ")?;
    let variable = if variable.trim().is_empty() {
        "SMITH_CHECKPOINT_SECRET".to_owned()
    } else {
        variable.trim().to_owned()
    };
    let reference = setup_environment_reference(&variable)
        .map_err(|error| anyhow::anyhow!(error))
        .context("validating checkpoint-key environment reference")?;
    refuse_unsafe_checkpoint_rotation(
        context,
        &ExistingCheckpointSource::Credential(reference.to_string()),
    )?;
    let patch = ConfigFile {
        persistence: Some(PersistenceSection {
            checkpoint_key: None,
            checkpoint_key_credential: Some(reference.to_string()),
            ..PersistenceSection::default()
        }),
        ..ConfigFile::default()
    };
    let outcome = commit_checkpoint_patch(&context.user_dir, patch)?;
    if outcome == SetupOutcome::Completed {
        println!(
            "Set {variable} to a 64-character hexadecimal key before starting Smith; no credential service will be queried."
        );
    }
    Ok(outcome)
}

fn setup_platform_checkpoint_key(context: &CheckpointSetupContext) -> Result<SetupOutcome> {
    if context.source == ExistingCheckpointSource::Platform {
        println!(
            "OS protected storage is already the default. This command did not query Keychain or Secret Service."
        );
        return Ok(SetupOutcome::Completed);
    }
    refuse_unsafe_checkpoint_rotation(context, &ExistingCheckpointSource::Platform)?;
    let prepared = prepare_checkpoint_key_source_removal(&context.user_dir)
        .context("preparing removal of the explicit checkpoint-key source")?;
    commit_prepared_checkpoint_edit(prepared)
}

fn commit_checkpoint_patch(user_dir: &Path, patch: ConfigFile) -> Result<SetupOutcome> {
    let prepared = prepare_user_config_edit(user_dir, &patch)
        .context("preparing the owner-only checkpoint-key configuration")?;
    commit_prepared_checkpoint_edit(prepared)
}

fn commit_prepared_checkpoint_edit(
    prepared: smith_config::user_config::PreparedConfigEdit,
) -> Result<SetupOutcome> {
    println!();
    print!("{}", prepared.preview());
    let collisions = !prepared.collisions().is_empty();
    if collisions
        && prompt_line("Replace the redacted existing checkpoint-key source? [y/N]: ")?.trim()
            != "y"
    {
        return Ok(SetupOutcome::Cancelled);
    }
    let committed = prepared
        .commit(collisions)
        .context("publishing checkpoint-key configuration atomically")?;
    committed.accept();
    println!("Checkpoint protection configuration committed.");
    Ok(SetupOutcome::Completed)
}

pub(super) fn refuse_unsafe_checkpoint_rotation(
    context: &CheckpointSetupContext,
    proposed: &ExistingCheckpointSource,
) -> Result<()> {
    if context.source == *proposed || !protected_checkpoint_exists(&context.sessions_dir)? {
        return Ok(());
    }
    anyhow::bail!(
        "checkpoint-key source change refused: protected checkpoints already exist under `{}`; resume or deliberately retire that state before rotating the key; configuration was not modified",
        context.sessions_dir.display()
    )
}

pub(super) fn protected_checkpoint_exists(sessions_dir: &Path) -> Result<bool> {
    if !sessions_dir.exists() {
        return Ok(false);
    }
    let mut pending = vec![sessions_dir.to_path_buf()];
    let mut inspected = 0usize;
    while let Some(directory) = pending.pop() {
        for entry in std::fs::read_dir(&directory).with_context(|| {
            format!(
                "inspecting `{}` for protected checkpoints",
                directory.display()
            )
        })? {
            let entry = entry.with_context(|| {
                format!(
                    "inspecting `{}` for protected checkpoints",
                    directory.display()
                )
            })?;
            inspected = inspected.saturating_add(1);
            if inspected > 100_000 {
                anyhow::bail!(
                    "checkpoint-key source change refused: the checkpoint inventory exceeds the safety scan limit"
                );
            }
            let file_type = entry.file_type().with_context(|| {
                format!(
                    "inspecting `{}` for protected checkpoints",
                    entry.path().display()
                )
            })?;
            if file_type.is_symlink() {
                continue;
            }
            if file_type.is_dir() {
                pending.push(entry.path());
            } else if file_type.is_file()
                && entry
                    .file_name()
                    .to_str()
                    .is_some_and(|name| name.ends_with(".checkpoint.bin"))
            {
                return Ok(true);
            }
        }
    }
    Ok(false)
}

fn prompt_line(prompt: &str) -> Result<String> {
    print!("{prompt}");
    io::stdout().flush().context("flushing setup prompt")?;
    let mut line = String::new();
    io::stdin()
        .read_line(&mut line)
        .context("reading setup response")?;
    Ok(line)
}

fn hex_encode(bytes: &[u8]) -> String {
    const HEX: &[u8; 16] = b"0123456789abcdef";
    let mut encoded = String::with_capacity(bytes.len() * 2);
    for byte in bytes {
        encoded.push(char::from(HEX[usize::from(byte >> 4)]));
        encoded.push(char::from(HEX[usize::from(byte & 0x0f)]));
    }
    encoded
}
