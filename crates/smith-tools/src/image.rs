//! Provider-backed image generation and editing.
//!
//! The HTTP implementation lives in `smith-runtime`; this module owns the
//! model-facing schema, reference validation, bounded image loading, and
//! private PNG storage.

use std::fmt;
use std::fs::{self, OpenOptions};
use std::io::{Read, Write};
use std::path::{Component, Path, PathBuf};
use std::sync::Arc;

use agent_runtime_core::content::ContentPart;
use agent_runtime_core::error::{ErrorKind, RuntimeError};
use agent_runtime_core::ids::SessionId;
use agent_runtime_core::tool::{
    InvocationContext, PreparationContext, PreparedToolCall, Tool, ToolCallDisplay, ToolEffects,
    ToolOutcome, ToolSpec,
};
use agent_runtime_core::workspace::Workspace;
use agent_runtime_registry::Permission;
use async_trait::async_trait;
use base64::Engine as _;
use cap_std::ambient_authority;
use cap_std::fs::Dir;
use image::{ImageFormat, ImageReader};
use serde_json::{Value, json};

use crate::support::invalid;

/// Largest accepted input reference image.
pub const MAX_REFERENCE_BYTES: usize = 20 * 1024 * 1024;
/// Largest encoded/decoded image API response accepted by Smith.
pub const MAX_GENERATED_PNG_BYTES: usize = 24 * 1024 * 1024;
/// Largest data URL accepted from recent conversation history per reference.
pub const MAX_RECENT_IMAGE_DATA_URL_BYTES: usize = 28 * 1024 * 1024;
const MAX_PROMPT_CHARS: usize = 4_000;

/// One reference image encoded for the Images API.
#[derive(Clone)]
pub struct ImageReference {
    /// Complete data URI, such as `data:image/png;base64,...`.
    pub data_url: String,
}

impl fmt::Debug for ImageReference {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("ImageReference([image data redacted])")
    }
}

/// One generation or edit operation.
#[derive(Clone)]
pub struct ImageGenerationRequest {
    /// The model-authored prompt.
    pub prompt: String,
    /// Selected image model.
    pub model: String,
    /// Provider image quality.
    pub quality: String,
    /// Provider image size.
    pub size: String,
    /// Empty for generation; one to five entries for edits.
    pub references: Vec<ImageReference>,
}

impl fmt::Debug for ImageGenerationRequest {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("ImageGenerationRequest")
            .field("prompt_chars", &self.prompt.chars().count())
            .field("model", &self.model)
            .field("quality", &self.quality)
            .field("size", &self.size)
            .field("reference_count", &self.references.len())
            .finish()
    }
}

/// The provider adapter used for one image request.
#[async_trait]
pub trait ImageGenerationBackend: Send + Sync + fmt::Debug {
    /// Calls generation or edit and returns the PNG bytes.
    async fn generate(
        &self,
        request: ImageGenerationRequest,
        ctx: &InvocationContext,
    ) -> Result<Vec<u8>, RuntimeError>;

    /// Exact endpoint used by the request, for prepared network authority.
    fn endpoint(&self) -> &str;
}

/// Reads recent image parts from the active canonical session history.
pub trait RecentImageSource: Send + Sync + fmt::Debug {
    /// Returns the newest `count` image URLs in chronological order.
    fn recent_images(&self, session: &SessionId, count: usize)
    -> Result<Vec<String>, RuntimeError>;
}

/// The `generate_image` built-in tool.
pub struct GenerateImageTool {
    backend: Arc<dyn ImageGenerationBackend>,
    recent: Arc<dyn RecentImageSource>,
    generated_dir: PathBuf,
    model: String,
    quality: String,
    size: String,
}

impl fmt::Debug for GenerateImageTool {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("GenerateImageTool")
            .field("endpoint", &self.backend.endpoint())
            .field("generated_dir", &self.generated_dir)
            .field("model", &self.model)
            .field("quality", &self.quality)
            .field("size", &self.size)
            .finish_non_exhaustive()
    }
}

impl GenerateImageTool {
    /// Creates the tool for one resolved provider and image configuration.
    pub fn new(
        backend: Arc<dyn ImageGenerationBackend>,
        recent: Arc<dyn RecentImageSource>,
        generated_dir: impl Into<PathBuf>,
        model: impl Into<String>,
        quality: impl Into<String>,
        size: impl Into<String>,
    ) -> Self {
        Self {
            backend,
            recent,
            generated_dir: generated_dir.into(),
            model: model.into(),
            quality: quality.into(),
            size: size.into(),
        }
    }
}

#[async_trait]
impl Tool for GenerateImageTool {
    fn spec(&self) -> ToolSpec {
        ToolSpec::new(
            "generate_image",
            "Generate an image from a prompt, or edit up to five workspace, generated-image, or recent conversation images.",
            json!({
                "type": "object",
                "properties": {
                    "prompt": { "type": "string", "description": "What the image should show or how the references should be changed." },
                    "reference_paths": {
                        "type": "array", "maxItems": 5,
                        "items": { "type": "string" },
                        "description": "Optional image paths inside the project or Smith's generated-image directory."
                    },
                    "recent_images": {
                        "type": "integer", "minimum": 1, "maximum": 5,
                        "description": "Use the newest 1–5 images in this session's conversation."
                    }
                },
                "required": ["prompt"],
                "additionalProperties": false
            }),
            image_effects(self.backend.endpoint(), true),
        )
        .with_permission_upper_bound(
            [
                Permission::HostFsRead,
                Permission::HostFsWrite,
                Permission::NetHttp,
                Permission::DataEgress,
            ]
            .into_iter()
            .collect(),
        )
    }

    async fn prepare(
        &self,
        mut arguments: Value,
        ctx: &PreparationContext,
    ) -> Result<PreparedToolCall, RuntimeError> {
        let prompt = arguments
            .get("prompt")
            .and_then(Value::as_str)
            .ok_or_else(|| invalid("`prompt` is required and must be a string"))?
            .to_owned();
        if prompt.trim().is_empty() || prompt.chars().count() > MAX_PROMPT_CHARS {
            return Err(invalid(format!(
                "`prompt` must contain 1 to {MAX_PROMPT_CHARS} characters"
            )));
        }
        let reference_paths = arguments.get("reference_paths").cloned();
        let recent_images = arguments.get("recent_images").cloned();
        if reference_paths.is_some() && recent_images.is_some() {
            return Err(invalid(
                "provide `reference_paths` or `recent_images`, not both",
            ));
        }
        let mut references = Vec::new();
        if let Some(paths) = reference_paths.as_ref() {
            let paths = paths
                .as_array()
                .ok_or_else(|| invalid("`reference_paths` must be an array of strings"))?;
            if paths.len() > 5 {
                return Err(invalid("`reference_paths` accepts at most five images"));
            }
            let workspace =
                smith_host::workspace::ProjectWorkspace::from_workspace(ctx.workspace.as_ref())
                    .ok_or_else(|| {
                        RuntimeError::new(
                            ErrorKind::Workspace,
                            "image references require Smith's ProjectWorkspace capability",
                        )
                    })?;
            let generated_root = canonical_generated_root(&self.generated_dir)?;
            for (index, path) in paths.iter().enumerate() {
                let path = path
                    .as_str()
                    .ok_or_else(|| invalid("each `reference_paths` item must be a string"))?;
                let resolved = resolve_reference_path(path, ctx, workspace, &generated_root)?;
                references.push(resolved);
                // The prepared arguments carry only canonical, bounded path
                // strings; the corresponding bytes are read after approval.
                arguments["reference_paths"][index] =
                    Value::String(references[index].to_string_lossy().into_owned());
            }
        } else if let Some(count) = recent_images.as_ref() {
            let count = count
                .as_u64()
                .and_then(|value| usize::try_from(value).ok())
                .filter(|value| (1..=5).contains(value))
                .ok_or_else(|| invalid("`recent_images` must be an integer from 1 to 5"))?;
            arguments["recent_images"] = Value::Number((count as u64).into());
        }

        let has_reference_paths = reference_paths.is_some();
        let effects = image_effects(self.backend.endpoint(), has_reference_paths);
        let (permissions, resource) =
            effects.authorization_request("generate_image", ctx.workspace.root());
        let display = ToolCallDisplay::new("Generate Image")
            .with_detail(prompt.chars().take(120).collect::<String>());
        Ok(PreparedToolCall::new(
            ctx.call_id.clone(),
            "generate_image",
            arguments,
            permissions,
            resource,
            effects,
            display,
        ))
    }

    async fn invoke(
        &self,
        prepared: PreparedToolCall,
        ctx: &InvocationContext,
    ) -> Result<ToolOutcome, RuntimeError> {
        if ctx.should_stop() {
            return Err(RuntimeError::cancelled(
                "image generation stopped before the request began",
            ));
        }
        let arguments = prepared.into_arguments();
        let prompt = arguments["prompt"].as_str().expect("validated prompt");
        let generated_root = canonical_generated_root(&self.generated_dir)?;
        let mut references = Vec::new();
        if let Some(paths) = arguments.get("reference_paths").and_then(Value::as_array) {
            let workspace =
                smith_host::workspace::ProjectWorkspace::from_workspace(ctx.workspace.as_ref())
                    .ok_or_else(|| {
                        RuntimeError::new(
                            ErrorKind::Workspace,
                            "image references require Smith's ProjectWorkspace capability",
                        )
                    })?
                    .clone();
            for path in paths {
                let raw = path.as_str().ok_or_else(|| invalid("invalid image path"))?;
                let bytes = read_reference(raw, &workspace, &generated_root)?;
                let mime = image_mime(&bytes)?;
                let data_url = format!(
                    "data:{mime};base64,{}",
                    base64::engine::general_purpose::STANDARD.encode(bytes)
                );
                references.push(ImageReference { data_url });
            }
        } else if let Some(count) = arguments
            .get("recent_images")
            .and_then(Value::as_u64)
            .and_then(|value| usize::try_from(value).ok())
        {
            for data_url in self.recent.recent_images(&ctx.session, count)? {
                if !data_url.starts_with("data:image/")
                    || data_url.len() > MAX_RECENT_IMAGE_DATA_URL_BYTES
                {
                    return Err(invalid(
                        "recent conversation image is unavailable or larger than 20 MiB",
                    ));
                }
                references.push(ImageReference { data_url });
            }
        }
        let request = ImageGenerationRequest {
            prompt: prompt.to_owned(),
            model: self.model.clone(),
            quality: self.quality.clone(),
            size: self.size.clone(),
            references,
        };
        let png = self.backend.generate(request, ctx).await?;
        if png.len() > MAX_GENERATED_PNG_BYTES {
            return Err(RuntimeError::new(
                ErrorKind::Provider,
                "generated image exceeded the 24 MiB limit",
            ));
        }
        let dimensions = image_dimensions(&png)?;
        let session = safe_component(ctx.session.as_str());
        let call = safe_component(ctx.call_id.as_str());
        let path = save_png(&generated_root, &session, &call, &png)?;
        let path_hint = path.to_string_lossy().into_owned();
        let data_url = format!(
            "data:image/png;base64,{}",
            base64::engine::general_purpose::STANDARD.encode(&png)
        );
        Ok(ToolOutcome {
            value: json!({"path": path_hint, "width": dimensions.0, "height": dimensions.1}),
            content: vec![
                ContentPart::Image {
                    url: data_url,
                    detail: Some("high".to_owned()),
                },
                ContentPart::text(format!(
                    "Saved image {} ({}×{} px)",
                    path.display(),
                    dimensions.0,
                    dimensions.1
                )),
            ]
            .into(),
            is_error: false,
        })
    }
}

fn image_effects(endpoint: &str, read_references: bool) -> ToolEffects {
    let effects = ToolEffects::new(Vec::new());
    let effects = if read_references {
        effects.with_host_read("smith:image-files")
    } else {
        effects
    };
    effects
        .with_host_write("smith:image-files", "smith:generated-images")
        .with_network_to(endpoint)
        .with_data_egress_to(endpoint)
}

fn resolve_reference_path(
    raw: &str,
    ctx: &PreparationContext,
    workspace: &smith_host::workspace::ProjectWorkspace,
    generated_root: &Path,
) -> Result<PathBuf, RuntimeError> {
    let raw_path = expand_generated_tilde(raw, generated_root);
    if raw_path.is_absolute() {
        let workspace_root = Path::new(ctx.workspace.root());
        if raw_path.starts_with(workspace_root) {
            let relative = workspace.relative_path(&raw_path)?;
            return Ok(workspace.display_path(relative));
        }
        let canonical = raw_path.canonicalize().map_err(|_| {
            RuntimeError::new(ErrorKind::Workspace, "image reference path does not exist")
        })?;
        if !canonical.starts_with(generated_root) {
            return Err(RuntimeError::new(
                ErrorKind::Workspace,
                "image reference must be inside the workspace or generated-image directory",
            ));
        }
        return Ok(canonical);
    }
    let resolved = ctx.workspace.resolve(raw)?;
    let relative = workspace.relative_path(&resolved)?;
    Ok(workspace.display_path(relative))
}

fn expand_generated_tilde(raw: &str, generated_root: &Path) -> PathBuf {
    if let Some(suffix) = raw.strip_prefix("~/.smith/generated_images/")
        && suffix
            .split('/')
            .all(|segment| !segment.is_empty() && segment != "." && segment != "..")
    {
        return generated_root.join(suffix);
    }
    PathBuf::from(raw)
}

fn canonical_generated_root(root: &Path) -> Result<PathBuf, RuntimeError> {
    if root
        .symlink_metadata()
        .is_ok_and(|metadata| metadata.file_type().is_symlink())
    {
        return Err(RuntimeError::new(
            ErrorKind::Workspace,
            "generated-image directory must not be a symlink",
        ));
    }
    if let Ok(canonical) = root.canonicalize() {
        return Ok(canonical);
    }
    if root.is_absolute() {
        Ok(root.to_path_buf())
    } else {
        std::env::current_dir()
            .map(|cwd| cwd.join(root))
            .map_err(|_| RuntimeError::new(ErrorKind::Workspace, "image directory unavailable"))
    }
}

fn read_reference(
    path: &str,
    workspace: &smith_host::workspace::ProjectWorkspace,
    generated_root: &Path,
) -> Result<Vec<u8>, RuntimeError> {
    let path = Path::new(path);
    if path.starts_with(workspace.root()) {
        let read = workspace.read_bounded(path, MAX_REFERENCE_BYTES)?;
        return Ok(read.bytes);
    }
    let canonical = path.canonicalize().map_err(|_| {
        RuntimeError::new(ErrorKind::Workspace, "image reference path does not exist")
    })?;
    let relative = canonical.strip_prefix(generated_root).map_err(|_| {
        RuntimeError::new(
            ErrorKind::Workspace,
            "image reference must be inside the workspace or generated-image directory",
        )
    })?;
    if relative
        .components()
        .any(|component| !matches!(component, Component::Normal(_)))
    {
        return Err(invalid("image reference path is not canonical"));
    }
    let directory = Dir::open_ambient_dir(generated_root, ambient_authority()).map_err(|_| {
        RuntimeError::new(
            ErrorKind::Workspace,
            "generated-image directory is unavailable",
        )
    })?;
    let file = directory.open(relative).map_err(|_| {
        RuntimeError::new(
            ErrorKind::Workspace,
            "image reference file could not be opened",
        )
    })?;
    let metadata = file.metadata().map_err(|_| {
        RuntimeError::new(
            ErrorKind::Workspace,
            "image reference file could not be inspected",
        )
    })?;
    if !metadata.is_file() || metadata.len() > MAX_REFERENCE_BYTES as u64 {
        return Err(invalid(
            "image reference must be a file no larger than 20 MiB",
        ));
    }
    let mut bytes = Vec::with_capacity(metadata.len() as usize);
    file.take((MAX_REFERENCE_BYTES + 1) as u64)
        .read_to_end(&mut bytes)
        .map_err(|_| {
            RuntimeError::new(ErrorKind::Workspace, "image reference could not be read")
        })?;
    if bytes.len() > MAX_REFERENCE_BYTES {
        return Err(invalid("image reference must be no larger than 20 MiB"));
    }
    Ok(bytes)
}

fn image_mime(bytes: &[u8]) -> Result<&'static str, RuntimeError> {
    if bytes.starts_with(b"\x89PNG\r\n\x1a\n") {
        Ok("image/png")
    } else if bytes.starts_with(&[0xff, 0xd8, 0xff]) {
        Ok("image/jpeg")
    } else if bytes.len() >= 12 && &bytes[..4] == b"RIFF" && &bytes[8..12] == b"WEBP" {
        Ok("image/webp")
    } else {
        Err(invalid(
            "image reference must be a PNG, JPEG, or WebP image",
        ))
    }
}

fn image_dimensions(bytes: &[u8]) -> Result<(u32, u32), RuntimeError> {
    let reader = ImageReader::with_format(std::io::Cursor::new(bytes), ImageFormat::Png);
    reader
        .into_dimensions()
        .map_err(|_| invalid("image provider response was not a valid PNG"))
}

fn safe_component(raw: &str) -> String {
    let sanitized: String = raw
        .chars()
        .filter(|ch| ch.is_ascii_alphanumeric() || matches!(ch, '_' | '-'))
        .take(80)
        .collect();
    if sanitized.is_empty() {
        "unknown".to_owned()
    } else {
        sanitized
    }
}

fn save_png(root: &Path, session: &str, call: &str, png: &[u8]) -> Result<PathBuf, RuntimeError> {
    let session_dir = root.join(session);
    #[cfg(unix)]
    {
        use std::os::unix::fs::{DirBuilderExt, OpenOptionsExt, PermissionsExt};
        for path in [root, session_dir.as_path()] {
            if path
                .symlink_metadata()
                .is_ok_and(|metadata| metadata.file_type().is_symlink())
            {
                return Err(RuntimeError::new(
                    ErrorKind::Workspace,
                    "generated-image path must not contain a symlink",
                ));
            }
        }
        let mut builder = fs::DirBuilder::new();
        builder.recursive(true).mode(0o700);
        builder.create(&session_dir).map_err(|_| {
            RuntimeError::new(
                ErrorKind::Workspace,
                "generated-image directory could not be created",
            )
        })?;
        fs::set_permissions(root, fs::Permissions::from_mode(0o700)).map_err(|_| {
            RuntimeError::new(
                ErrorKind::Workspace,
                "generated-image permissions could not be set",
            )
        })?;
        fs::set_permissions(&session_dir, fs::Permissions::from_mode(0o700)).map_err(|_| {
            RuntimeError::new(
                ErrorKind::Workspace,
                "generated-image permissions could not be set",
            )
        })?;
        let path = session_dir.join(format!("{call}.png"));
        let mut file = OpenOptions::new()
            .write(true)
            .create_new(true)
            .mode(0o600)
            .open(&path)
            .map_err(|_| {
                RuntimeError::new(ErrorKind::Workspace, "generated image could not be saved")
            })?;
        file.write_all(png)
            .and_then(|()| file.sync_all())
            .map_err(|_| {
                RuntimeError::new(ErrorKind::Workspace, "generated image could not be saved")
            })?;
        Ok(path)
    }
    #[cfg(not(unix))]
    {
        fs::create_dir_all(&session_dir).map_err(|_| {
            RuntimeError::new(
                ErrorKind::Workspace,
                "generated-image directory could not be created",
            )
        })?;
        let path = session_dir.join(format!("{call}.png"));
        let mut file = OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&path)
            .map_err(|_| {
                RuntimeError::new(ErrorKind::Workspace, "generated image could not be saved")
            })?;
        file.write_all(png)
            .and_then(|()| file.sync_all())
            .map_err(|_| {
                RuntimeError::new(ErrorKind::Workspace, "generated image could not be saved")
            })?;
        Ok(path)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Arc;

    use agent_runtime_core::cancel::Cancellation;
    use agent_runtime_core::clock::{Deadline, SystemClock};
    use agent_runtime_core::content::ContentPart;
    use agent_runtime_core::ids::{RequestId, ToolCallId};
    use agent_runtime_core::tool::{InvocationContext, PreparationContext};
    use smith_host::workspace::ProjectWorkspace;

    #[test]
    fn filenames_are_sanitized_to_one_component() {
        assert_eq!(
            safe_component("../session:with spaces"),
            "sessionwithspaces"
        );
        assert_eq!(safe_component("../../"), "unknown");
    }

    #[test]
    fn mime_is_selected_from_image_magic_bytes() {
        assert_eq!(image_mime(b"\x89PNG\r\n\x1a\nrest").unwrap(), "image/png");
        assert_eq!(image_mime(&[0xff, 0xd8, 0xff, 0]).unwrap(), "image/jpeg");
        assert_eq!(image_mime(b"RIFF0000WEBP").unwrap(), "image/webp");
        assert!(image_mime(b"text").is_err());
    }

    #[derive(Debug)]
    struct FixedImageBackend(Vec<u8>);

    #[async_trait]
    impl ImageGenerationBackend for FixedImageBackend {
        async fn generate(
            &self,
            _request: ImageGenerationRequest,
            _ctx: &InvocationContext,
        ) -> Result<Vec<u8>, RuntimeError> {
            Ok(self.0.clone())
        }

        fn endpoint(&self) -> &str {
            "https://api.openai.com/v1"
        }
    }

    #[derive(Debug)]
    struct NoRecentImages;

    impl RecentImageSource for NoRecentImages {
        fn recent_images(
            &self,
            _session: &SessionId,
            _count: usize,
        ) -> Result<Vec<String>, RuntimeError> {
            Ok(Vec::new())
        }
    }

    #[tokio::test]
    async fn generation_saves_private_png_and_returns_image_path_hint() {
        let temporary = tempfile::tempdir().expect("temporary directory");
        let workspace_path = temporary.path().join("project");
        fs::create_dir_all(&workspace_path).expect("workspace directory");
        let workspace = Arc::new(ProjectWorkspace::new(&workspace_path).expect("workspace"));
        let generated_dir = temporary.path().join("generated_images");
        let mut png = std::io::Cursor::new(Vec::new());
        image::DynamicImage::new_rgba8(2, 3)
            .write_to(&mut png, ImageFormat::Png)
            .expect("encode test PNG");
        let png = png.into_inner();
        let tool = GenerateImageTool::new(
            Arc::new(FixedImageBackend(png.clone())),
            Arc::new(NoRecentImages),
            &generated_dir,
            "gpt-image-2",
            "auto",
            "auto",
        );
        let session = SessionId::new("session/../unsafe");
        let call_id = ToolCallId::new("call/../image");
        let clock = Arc::new(SystemClock);
        let prepared = tool
            .prepare(
                json!({"prompt":"A small blue bird"}),
                &PreparationContext {
                    session: session.clone(),
                    turn: None,
                    call_id: call_id.clone(),
                    request: RequestId::new("request"),
                    workspace: workspace.clone(),
                    clock: clock.clone(),
                    cancel: Cancellation::new(),
                    deadline: Deadline::never(),
                },
            )
            .await
            .expect("prepare image tool");
        let outcome = tool
            .invoke(
                prepared,
                &InvocationContext {
                    session,
                    turn: None,
                    call_id,
                    request: RequestId::new("request"),
                    workspace,
                    clock,
                    cancel: Cancellation::new(),
                    deadline: Deadline::never(),
                    output_limit: 4096,
                },
            )
            .await
            .expect("generate image");

        let path = generated_dir.join("sessionunsafe").join("callimage.png");
        assert_eq!(outcome.value["path"], path.to_string_lossy().as_ref());
        assert_eq!(outcome.value["width"], 2);
        assert_eq!(outcome.value["height"], 3);
        assert_eq!(fs::read(&path).expect("saved image"), png);
        let parts = outcome.content.as_inline().expect("inline image and hint");
        assert!(
            parts
                .iter()
                .any(|part| matches!(part, ContentPart::Image { .. }))
        );
        assert!(parts.iter().any(|part| matches!(
            part,
            ContentPart::Text { text }
                if text.contains("callimage.png") && text.contains("2×3 px")
        )));
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            assert_eq!(
                fs::metadata(path)
                    .expect("image metadata")
                    .permissions()
                    .mode()
                    & 0o777,
                0o600
            );
        }
    }
}
