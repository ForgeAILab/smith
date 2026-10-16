//! Prepare image files once, before they enter conversation history.

use std::borrow::Cow;
use std::fmt;
use std::fs::File;
use std::io::{self, Cursor, Read};
use std::path::{Path, PathBuf};

use base64::Engine as _;
use base64::engine::general_purpose::STANDARD;
use image::codecs::jpeg::JpegEncoder;
use image::codecs::png::PngEncoder;
use image::codecs::webp::WebPEncoder;
use image::imageops::FilterType;
use image::{DynamicImage, ImageDecoder, ImageEncoder, ImageFormat, ImageReader};

/// Input guard, independent of provider upload limits.
pub const MAX_IMAGE_FILE_BYTES: u64 = 64 * 1024 * 1024;
/// Checked from the header before allocating decoded pixels.
pub const MAX_IMAGE_PIXELS: u64 = 100_000_000;
const TILE_SIDE: u32 = 32;

/// Provider family of the active main model. ChatGPT uses `OpenAi`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ImageProvider {
    /// OpenAI API and ChatGPT.
    OpenAi,
    /// Direct Anthropic API.
    Anthropic,
    /// Google Gemini.
    Gemini,
    /// xAI.
    Xai,
    /// Compatible and unknown providers.
    Other,
}

/// Whether an upload limit counts encoded image bytes or their base64 payload.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum EncodedSizeLimit {
    /// Counts raw encoded image bytes.
    Bytes(u64),
    /// Counts base64 characters, including padding but excluding the data URL prefix.
    Base64Bytes(u64),
}

impl EncodedSizeLimit {
    fn fits(self, byte_len: usize) -> bool {
        let byte_len = byte_len as u64;
        match self {
            Self::Bytes(max) => byte_len <= max,
            Self::Base64Bytes(max) => byte_len.div_ceil(3) * 4 <= max,
        }
    }
}

/// Limits for one image; callers still account for aggregate request limits.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ImageLimits {
    /// Supported source formats; GIFs are flattened even when listed here.
    pub formats: &'static [ImageFormat],
    /// Maximum width or height in pixels.
    pub max_long_edge: u32,
    /// Maximum number of 32×32 tiles, rounding up on both axes.
    pub tile_budget: Option<u32>,
    /// Per-image payload allowance; `None` means no per-image cap.
    pub max_encoded_bytes: Option<EncodedSizeLimit>,
}

impl ImageLimits {
    /// Approved provider table, checked 2026-10-15 (design.md, D2).
    pub fn for_provider(provider: ImageProvider) -> Self {
        use ImageFormat::{Gif, Jpeg, Png, WebP};
        match provider {
            // OpenAI "Images and vision": formats, 512 MB request cap (no
            // per-image cap). Codex core/image_preparation.rs HIGH_DETAIL_LIMITS:
            // 2048 px and 2,500 tiles. GIF is always flattened, as in Codex.
            ImageProvider::OpenAi => Self {
                formats: &[Png, Jpeg, WebP, Gif],
                max_long_edge: 2048,
                tile_budget: Some(2_500),
                max_encoded_bytes: None,
            },
            // Anthropic "Vision": formats, 2576 px high-resolution edge and
            // 10 MB base64. Bedrock/Vertex's 5 MB needs an endpoint override.
            ImageProvider::Anthropic => Self {
                formats: &[Png, Jpeg, Gif, WebP],
                max_long_edge: 2576,
                tile_budget: None,
                max_encoded_bytes: Some(EncodedSizeLimit::Base64Bytes(10_000_000)),
            },
            // Gemini "Image understanding": formats and 20 MB inline request
            // cap, conservatively counted as base64. D2 chooses the 3072 px
            // edge; the documentation does not specify a pixel edge.
            ImageProvider::Gemini => Self {
                formats: &[Png, Jpeg, WebP],
                max_long_edge: 3072,
                tile_budget: None,
                max_encoded_bytes: Some(EncodedSizeLimit::Base64Bytes(20_000_000)),
            },
            // xAI "Image Understanding": PNG/JPEG, 2048 px, 20 MiB per image.
            ImageProvider::Xai => Self {
                formats: &[Png, Jpeg],
                max_long_edge: 2048,
                tile_budget: None,
                max_encoded_bytes: Some(EncodedSizeLimit::Bytes(20 * 1024 * 1024)),
            },
            // D2's conservative fallback policy for compatible/unknown APIs.
            ImageProvider::Other => Self {
                formats: &[Png, Jpeg],
                max_long_edge: 2048,
                tile_budget: None,
                max_encoded_bytes: Some(EncodedSizeLimit::Base64Bytes(5_000_000)),
            },
        }
    }
}

/// Dimensions, file format and raw encoded byte count (before base64).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ImageInfo {
    /// Pixel width.
    pub width: u32,
    /// Pixel height.
    pub height: u32,
    /// Detected or emitted file format.
    pub format: ImageFormat,
    /// Raw encoded bytes, excluding base64 expansion.
    pub byte_len: usize,
}

/// The model input and the original/sent metadata for its description.
#[derive(Clone)]
pub struct PreparedImage {
    /// Complete `data:<mime>;base64,...` input.
    pub data_url: String,
    /// MIME type of the sent image.
    pub media_type: &'static str,
    /// Input metadata before any transformation.
    pub original: ImageInfo,
    /// Metadata of the image in `data_url`.
    pub sent: ImageInfo,
    /// Whether the bytes sent differ from the input.
    pub changed: bool,
}

impl fmt::Debug for PreparedImage {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("PreparedImage")
            .field("media_type", &self.media_type)
            .field("original", &self.original)
            .field("sent", &self.sent)
            .field("changed", &self.changed)
            .finish_non_exhaustive()
    }
}

impl PreparedImage {
    /// A data-free line suitable for tool output and attachment displays.
    pub fn describe(&self, path_display: &str) -> String {
        let original = if self.changed {
            format!("{}×{} → ", self.original.width, self.original.height)
        } else {
            String::new()
        };
        format!(
            "{path_display} · {original}{}×{} {} · {} KiB",
            self.sent.width,
            self.sent.height,
            format_name(self.sent.format),
            self.sent.byte_len.div_ceil(1024),
        )
    }
}

/// `NotAnImage` lets callers retain their existing text/binary handling.
#[derive(Debug)]
pub enum ImageInputError {
    /// No recognized image signature; callers may try text/binary handling.
    NotAnImage,
    /// A recognized format is unsupported or fails decoding/encoding.
    UnsupportedOrCorrupt {
        /// Details from format validation or the codec.
        reason: String,
    },
    /// Input exceeded the file-size guard.
    FileTooLarge {
        /// Observed size in bytes.
        size: u64,
        /// Maximum accepted bytes.
        max: u64,
    },
    /// Header dimensions exceeded the pixel guard before decoding.
    TooManyPixels {
        /// Header width.
        width: u32,
        /// Header height.
        height: u32,
        /// Maximum accepted pixels.
        max: u64,
    },
    /// Even a 1×1 image cannot fit, or the supplied dimension budget is zero.
    CannotFitLimits,
    /// Opening, inspecting or reading a path failed.
    Io {
        /// Caller-supplied path.
        path: PathBuf,
        /// Underlying filesystem failure.
        source: io::Error,
    },
}

impl fmt::Display for ImageInputError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::NotAnImage => formatter.write_str("file is not a recognized image"),
            Self::UnsupportedOrCorrupt { reason } => {
                write!(formatter, "unsupported or corrupt image: {reason}")
            }
            Self::FileTooLarge { size, max } => write!(
                formatter,
                "image file is too large ({size} bytes); limit is {} MiB",
                max / (1024 * 1024)
            ),
            Self::TooManyPixels { width, height, max } => write!(
                formatter,
                "image has too many pixels ({width}×{height}); limit is {} megapixels",
                max / 1_000_000
            ),
            Self::CannotFitLimits => formatter.write_str("image cannot fit the provider's limits"),
            Self::Io { path, source } => {
                write!(
                    formatter,
                    "could not read image `{}`: {source}",
                    path.display()
                )
            }
        }
    }
}

impl std::error::Error for ImageInputError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Self::Io { source, .. } => Some(source),
            _ => None,
        }
    }
}

/// Reads a caller-authorized path, checking handle metadata before reading.
/// Workspace confinement/approval, where needed, belongs to the caller.
pub fn prepare_image_path(
    path: &Path,
    limits: &ImageLimits,
) -> Result<PreparedImage, ImageInputError> {
    let read = || -> io::Result<_> {
        let file = File::open(path)?;
        let size = file.metadata()?.len();
        Ok((file, size))
    };
    let io_error = |source| ImageInputError::Io {
        path: path.to_owned(),
        source,
    };
    let (file, size) = read().map_err(io_error)?;
    check_file_size(size)?;
    let mut bytes = Vec::new();
    // Also bound a file that grows after the metadata check.
    file.take(MAX_IMAGE_FILE_BYTES + 1)
        .read_to_end(&mut bytes)
        .map_err(io_error)?;
    prepare_image(&bytes, limits)
}

/// Validates, fits and encodes an image, without filesystem access.
pub fn prepare_image(bytes: &[u8], limits: &ImageLimits) -> Result<PreparedImage, ImageInputError> {
    check_file_size(bytes.len() as u64)?;
    let format = image::guess_format(bytes).map_err(|_| ImageInputError::NotAnImage)?;
    if !matches!(
        format,
        ImageFormat::Png | ImageFormat::Jpeg | ImageFormat::Gif | ImageFormat::WebP
    ) {
        return Err(invalid_image(format!("{format:?} input is not supported")));
    }

    // Some codecs apply an allocation limit while opening a header, before
    // exposing dimensions. Inspect the bounded header first so oversized
    // images report the pixel guard rather than a generic codec limit.
    if let Some((width, height)) = crate::image_header::image_dimensions(bytes) {
        check_pixel_count(width, height)?;
    }
    let decoder = ImageReader::with_format(Cursor::new(bytes), format)
        .into_decoder()
        .map_err(invalid_image)?;
    let (width, height) = decoder.dimensions();
    check_pixel_count(width, height)?;
    // Decode even for passthrough, so a valid signature with corrupt pixels
    // cannot enter history. GIF's decoder reads just the first frame.
    let mut image = DynamicImage::from_decoder(decoder).map_err(invalid_image)?;
    let original = ImageInfo {
        width,
        height,
        format,
        byte_len: bytes.len(),
    };
    let target = fitted_dimensions(width, height, limits)?;
    let resized = target != (width, height);
    if resized {
        image = image.resize_exact(target.0, target.1, FilterType::Triangle);
    }
    // Like Codex, flatten even still GIFs rather than forwarding animation.
    let mut sent_format = if format != ImageFormat::Gif && limits.formats.contains(&format) {
        format
    } else {
        ImageFormat::Png
    };
    let mut encoded = if !resized && sent_format == format {
        Cow::Borrowed(bytes)
    } else {
        Cow::Owned(encode_image(&image, sent_format)?)
    };
    if let Some(max) = limits.max_encoded_bytes {
        if !max.fits(encoded.len()) && sent_format == ImageFormat::Png && !has_transparency(&image)
        {
            sent_format = ImageFormat::Jpeg;
            encoded = Cow::Owned(encode_image(&image, sent_format)?);
        }
        while !max.fits(encoded.len()) {
            let next = (
                (image.width() * 3 / 4).max(1),
                (image.height() * 3 / 4).max(1),
            );
            if next == (image.width(), image.height()) {
                return Err(ImageInputError::CannotFitLimits);
            }
            image = image.resize_exact(next.0, next.1, FilterType::Triangle);
            encoded = Cow::Owned(encode_image(&image, sent_format)?);
        }
    }
    let media_type = sent_format.to_mime_type();
    Ok(PreparedImage {
        data_url: format!("data:{media_type};base64,{}", STANDARD.encode(&encoded)),
        media_type,
        original,
        sent: ImageInfo {
            width: image.width(),
            height: image.height(),
            format: sent_format,
            byte_len: encoded.len(),
        },
        changed: encoded.as_ref() != bytes,
    })
}

fn check_file_size(size: u64) -> Result<(), ImageInputError> {
    if size > MAX_IMAGE_FILE_BYTES {
        return Err(ImageInputError::FileTooLarge {
            size,
            max: MAX_IMAGE_FILE_BYTES,
        });
    }
    Ok(())
}

fn check_pixel_count(width: u32, height: u32) -> Result<(), ImageInputError> {
    if u64::from(width) * u64::from(height) > MAX_IMAGE_PIXELS {
        return Err(ImageInputError::TooManyPixels {
            width,
            height,
            max: MAX_IMAGE_PIXELS,
        });
    }
    Ok(())
}

fn invalid_image(reason: impl fmt::Display) -> ImageInputError {
    ImageInputError::UnsupportedOrCorrupt {
        reason: reason.to_string(),
    }
}

fn fitted_dimensions(
    width: u32,
    height: u32,
    limits: &ImageLimits,
) -> Result<(u32, u32), ImageInputError> {
    if width == 0 || height == 0 || limits.max_long_edge == 0 || limits.tile_budget == Some(0) {
        return Err(ImageInputError::CannotFitLimits);
    }
    let edge_scale = (f64::from(limits.max_long_edge) / f64::from(width.max(height))).min(1.0);
    let width = ((f64::from(width) * edge_scale).round() as u32).max(1);
    let height = ((f64::from(height) * edge_scale).round() as u32).max(1);
    let Some(budget) = limits.tile_budget else {
        return Ok((width, height));
    };
    if tile_count(width, height) <= u64::from(budget) {
        return Ok((width, height));
    }
    // Codex's patch math: fit area, then round the scaled tile grid down.
    let mut scale =
        (f64::from(TILE_SIDE).powi(2) * f64::from(budget) / f64::from(width) / f64::from(height))
            .sqrt();
    let tiles_wide = f64::from(width) * scale / f64::from(TILE_SIDE);
    let tiles_high = f64::from(height) * scale / f64::from(TILE_SIDE);
    scale *= (tiles_wide.floor() / tiles_wide).min(tiles_high.floor() / tiles_high);
    Ok((
        ((f64::from(width) * scale).floor() as u32).max(1),
        ((f64::from(height) * scale).floor() as u32).max(1),
    ))
}

fn tile_count(width: u32, height: u32) -> u64 {
    u64::from(width.div_ceil(TILE_SIDE)) * u64::from(height.div_ceil(TILE_SIDE))
}

fn has_transparency(image: &DynamicImage) -> bool {
    image.color().has_alpha() && image.to_rgba16().pixels().any(|pixel| pixel[3] != u16::MAX)
}

fn encode_image(image: &DynamicImage, format: ImageFormat) -> Result<Vec<u8>, ImageInputError> {
    let mut bytes = Vec::new();
    match format {
        ImageFormat::Jpeg => JpegEncoder::new_with_quality(&mut bytes, 85)
            .encode_image(&image.to_rgb8())
            .map_err(invalid_image)?,
        ImageFormat::Png => {
            let rgba = image.to_rgba8();
            PngEncoder::new(&mut bytes)
                .write_image(
                    rgba.as_raw(),
                    image.width(),
                    image.height(),
                    image::ColorType::Rgba8.into(),
                )
                .map_err(invalid_image)?;
        }
        ImageFormat::WebP => {
            let rgba = image.to_rgba8();
            WebPEncoder::new_lossless(&mut bytes)
                .write_image(
                    rgba.as_raw(),
                    image.width(),
                    image.height(),
                    image::ColorType::Rgba8.into(),
                )
                .map_err(invalid_image)?;
        }
        _ => return Err(invalid_image("unsupported output format")),
    }
    Ok(bytes)
}

fn format_name(format: ImageFormat) -> &'static str {
    match format {
        ImageFormat::Png => "PNG",
        ImageFormat::Jpeg => "JPEG",
        ImageFormat::Gif => "GIF",
        ImageFormat::WebP => "WebP",
        _ => "image",
    }
}

#[cfg(test)]
#[path = "image_preparation_tests.rs"]
mod tests;
