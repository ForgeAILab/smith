use super::*;

use image::codecs::gif::{GifEncoder, Repeat};
use image::{Frame, GenericImageView, Rgb, RgbImage, Rgba, RgbaImage};

fn limits() -> ImageLimits {
    ImageLimits::for_provider(ImageProvider::OpenAi)
}

fn payload(image: &PreparedImage) -> Vec<u8> {
    let (prefix, data) = image.data_url.split_once(',').unwrap();
    assert_eq!(prefix, format!("data:{};base64", image.media_type));
    let bytes = STANDARD.decode(data).unwrap();
    assert_eq!(bytes.len(), image.sent.byte_len);
    let decoded = image::load_from_memory(&bytes).unwrap();
    assert_eq!(decoded.dimensions(), (image.sent.width, image.sent.height));
    assert_eq!(image::guess_format(&bytes).unwrap(), image.sent.format);
    bytes
}

fn noise(width: u32, height: u32, transparent: bool) -> DynamicImage {
    let mut state = 1u32;
    DynamicImage::ImageRgba8(RgbaImage::from_fn(width, height, |_, _| {
        // Deterministic noise keeps compression tests reproducible.
        state ^= state << 13;
        state ^= state >> 17;
        state ^= state << 5;
        let [r, g, b, a] = state.to_le_bytes();
        Rgba([r, g, b, if transparent { a } else { 255 }])
    }))
}

#[test]
fn small_png_passes_through_byte_for_byte() {
    let bytes = encode_image(&noise(60, 40, false), ImageFormat::Png).unwrap();
    let prepared = prepare_image(&bytes, &limits()).unwrap();
    assert!(!prepared.changed);
    assert_eq!(prepared.original, prepared.sent);
    assert_eq!(prepared.original.byte_len, bytes.len());
    assert_eq!(prepared.original.format, ImageFormat::Png);
    assert_eq!(payload(&prepared), bytes);
    assert_eq!(
        prepared.describe("photo.png"),
        format!("photo.png · 60×40 PNG · {} KiB", bytes.len().div_ceil(1024))
    );
    assert!(!format!("{prepared:?}").contains("base64"));
}

#[test]
fn image_preparation_header_fits_without_decoding_pixels() {
    let bytes = encode_image(&noise(60, 40, false), ImageFormat::Png).unwrap();
    assert!(image_fits(&bytes, &limits()));
    // Only the header remains: this cannot be decoded as a complete image.
    assert!(image::load_from_memory(&bytes[..24]).is_err());
    assert!(image_fits(&bytes[..24], &limits()));
    assert!(!image_fits(
        &bytes,
        &ImageLimits {
            max_long_edge: 30,
            ..limits()
        }
    ));
    assert!(!image_fits(
        &bytes,
        &ImageLimits {
            tile_budget: Some(1),
            ..limits()
        }
    ));
    for max in [EncodedSizeLimit::Bytes(1), EncodedSizeLimit::Base64Bytes(1)] {
        assert!(!image_fits(
            &bytes,
            &ImageLimits {
                max_encoded_bytes: Some(max),
                ..limits()
            }
        ));
    }
    let gif = b"GIF89a\x02\0\x03\0";
    assert!(!image_fits(gif, &limits()));
}

#[test]
fn image_preparation_data_url_preserves_fits_and_is_deterministic() {
    let bytes = encode_image(&noise(60, 40, false), ImageFormat::WebP).unwrap();
    let url = format!("data:image/webp;base64,{}", STANDARD.encode(&bytes));
    assert_eq!(prepare_image_data_url(&url, &limits()).unwrap(), url);
    let xai = ImageLimits::for_provider(ImageProvider::Xai);
    let prepared = prepare_image_data_url(&url, &xai).unwrap();
    assert!(prepared.starts_with("data:image/png;base64,"));
    assert_eq!(prepare_image_data_url(&url, &xai).unwrap(), prepared);
    for bad in [
        "data:image/png;base64,!!!",
        "data:image/png,foo",
        "data:image/png;base64,AAAA",
    ] {
        assert!(prepare_image_data_url(bad, &xai).is_err());
    }
}

#[test]
fn large_jpeg_fits_each_providers_longest_side() {
    let source = DynamicImage::ImageRgb8(RgbImage::from_pixel(4096, 2048, Rgb([91, 131, 173])));
    let bytes = encode_image(&source, ImageFormat::Jpeg).unwrap();
    for (provider, edge) in [
        (ImageProvider::OpenAi, 2048),
        (ImageProvider::Anthropic, 2576),
        (ImageProvider::Gemini, 3072),
        (ImageProvider::Xai, 2048),
        (ImageProvider::Other, 2048),
    ] {
        let limits = ImageLimits::for_provider(provider);
        let prepared = prepare_image(&bytes, &limits).unwrap();
        assert!(prepared.changed, "{provider:?}");
        assert_eq!(
            (prepared.original.width, prepared.original.height),
            (4096, 2048)
        );
        assert_eq!(
            (prepared.sent.width, prepared.sent.height),
            (edge, edge / 2)
        );
        assert_eq!(prepared.sent.format, ImageFormat::Jpeg);
        if let Some(max) = limits.max_encoded_bytes {
            assert!(max.fits(prepared.sent.byte_len));
        }
        payload(&prepared);
    }
}

#[test]
fn tile_budget_uses_ceil_on_both_axes() {
    let bytes = encode_image(&noise(2048, 2048, false), ImageFormat::Png).unwrap();
    let prepared = prepare_image(&bytes, &limits()).unwrap();
    assert_eq!((prepared.sent.width, prepared.sent.height), (1600, 1600));
    assert_eq!(tile_count(prepared.sent.width, prepared.sent.height), 2500);
    assert!(prepared.changed);
    assert_eq!(
        fitted_dimensions(4032, 3024, &limits()).unwrap(),
        (1824, 1368)
    );
    assert_eq!(
        fitted_dimensions(1600, 1600, &limits()).unwrap(),
        (1600, 1600)
    );
    // A 1601 px edge uses 51 tiles, not 50.
    let target = fitted_dimensions(1601, 1600, &limits()).unwrap();
    assert!(tile_count(target.0, target.1) <= 2500);
    payload(&prepared);
}

#[test]
fn webp_becomes_png_for_xai_but_passes_through_for_openai() {
    let bytes = encode_image(&noise(48, 32, false), ImageFormat::WebP).unwrap();
    let prepared = prepare_image(&bytes, &ImageLimits::for_provider(ImageProvider::Xai)).unwrap();
    assert_eq!(prepared.original.format, ImageFormat::WebP);
    assert_eq!(prepared.sent.format, ImageFormat::Png);
    assert_eq!(prepared.media_type, "image/png");
    assert!(prepared.changed);
    let output = payload(&prepared);
    assert_eq!(
        image::load_from_memory(&output).unwrap().to_rgba8(),
        image::load_from_memory(&bytes).unwrap().to_rgba8()
    );
    let passthrough = prepare_image(&bytes, &limits()).unwrap();
    assert!(!passthrough.changed);
    assert_eq!(payload(&passthrough), bytes);
}

#[test]
fn animated_gif_becomes_its_first_frame_for_every_provider() {
    let first = RgbaImage::from_pixel(24, 16, Rgba([200, 20, 30, 255]));
    let second = RgbaImage::from_pixel(24, 16, Rgba([10, 200, 30, 255]));
    let mut bytes = Vec::new();
    {
        let mut encoder = GifEncoder::new(&mut bytes);
        encoder.set_repeat(Repeat::Infinite).unwrap();
        encoder.encode_frame(Frame::new(first.clone())).unwrap();
        encoder.encode_frame(Frame::new(second)).unwrap();
    }
    for provider in [
        ImageProvider::OpenAi,
        ImageProvider::Anthropic,
        ImageProvider::Gemini,
        ImageProvider::Xai,
        ImageProvider::Other,
    ] {
        let prepared = prepare_image(&bytes, &ImageLimits::for_provider(provider)).unwrap();
        assert_eq!(prepared.original.format, ImageFormat::Gif);
        assert_eq!(prepared.sent.format, ImageFormat::Png);
        assert!(prepared.changed);
        assert_eq!(
            image::load_from_memory(&payload(&prepared))
                .unwrap()
                .to_rgba8(),
            first
        );
    }
}

#[test]
fn opaque_png_over_base64_limit_becomes_quality_85_jpeg() {
    // RGBA with every alpha=255 must be treated as opaque too.
    let source = noise(160, 120, false);
    let bytes = encode_image(&source, ImageFormat::Png).unwrap();
    let jpeg = encode_image(&source, ImageFormat::Jpeg).unwrap();
    assert!(jpeg.len() < bytes.len());
    let mut limits = limits();
    limits.max_encoded_bytes = Some(EncodedSizeLimit::Base64Bytes(
        jpeg.len().div_ceil(3) as u64 * 4,
    ));
    let prepared = prepare_image(&bytes, &limits).unwrap();
    assert_eq!(prepared.sent.format, ImageFormat::Jpeg);
    assert_eq!((prepared.sent.width, prepared.sent.height), (160, 120));
    assert_eq!(payload(&prepared), jpeg);
    assert!(prepared.changed);
    assert_eq!(
        prepared.describe("photo.png"),
        format!(
            "photo.png · 160×120 → 160×120 JPEG · {} KiB",
            jpeg.len().div_ceil(1024)
        )
    );
    // Raw bytes would fit this cap, but base64 does not.
    limits.max_encoded_bytes = Some(EncodedSizeLimit::Base64Bytes(bytes.len() as u64));
    assert_eq!(
        prepare_image(&bytes, &limits).unwrap().sent.format,
        ImageFormat::Jpeg
    );
}

#[test]
fn transparent_png_stays_png_and_shrinks_in_quarter_steps() {
    let source = noise(160, 120, true);
    let bytes = encode_image(&source, ImageFormat::Png).unwrap();
    let first_step = source.resize_exact(120, 90, FilterType::Triangle);
    let expected = encode_image(&first_step, ImageFormat::Png).unwrap();
    assert!(expected.len() < bytes.len());
    let mut limits = limits();
    limits.max_encoded_bytes = Some(EncodedSizeLimit::Bytes(expected.len() as u64));
    let prepared = prepare_image(&bytes, &limits).unwrap();
    assert_eq!(prepared.sent.format, ImageFormat::Png);
    assert_eq!((prepared.sent.width, prepared.sent.height), (120, 90));
    assert_eq!(payload(&prepared), expected);
    assert!(has_transparency(
        &image::load_from_memory(&payload(&prepared)).unwrap()
    ));

    limits.max_encoded_bytes = Some(EncodedSizeLimit::Bytes(1000));
    let smaller = prepare_image(&bytes, &limits).unwrap();
    assert!(smaller.sent.byte_len <= 1000);
    assert!(smaller.sent.width < 120 && smaller.sent.height < 90);
    assert_eq!(smaller.sent.format, ImageFormat::Png);
    payload(&smaller);
}

#[test]
fn jpeg_over_byte_limit_keeps_shrinking_after_png_conversion() {
    let source = noise(160, 120, false);
    let mut limits = limits();
    limits.max_encoded_bytes = Some(EncodedSizeLimit::Bytes(2000));
    for format in [ImageFormat::Png, ImageFormat::Jpeg] {
        let bytes = encode_image(&source, format).unwrap();
        let prepared = prepare_image(&bytes, &limits).unwrap();
        assert_eq!(prepared.sent.format, ImageFormat::Jpeg);
        assert!(prepared.sent.byte_len <= 2000);
        assert!(prepared.sent.width < 160);
        payload(&prepared);
    }
}

#[test]
fn oversized_file_is_rejected_from_metadata_without_reading_64_mib() {
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("large.png");
    let file = File::create(&path).unwrap();
    // Sparse file: no image payload or 64 MiB allocation.
    file.set_len(MAX_IMAGE_FILE_BYTES + 1).unwrap();
    let error = prepare_image_path(&path, &limits()).unwrap_err();
    assert!(
        matches!(error, ImageInputError::FileTooLarge { size, max } if size == MAX_IMAGE_FILE_BYTES + 1 && max == MAX_IMAGE_FILE_BYTES)
    );
    assert!(error.to_string().contains("64 MiB"));
    assert!(check_file_size(MAX_IMAGE_FILE_BYTES).is_ok());
}

fn crc32(bytes: &[u8]) -> u32 {
    let mut crc = u32::MAX;
    for byte in bytes {
        crc ^= u32::from(*byte);
        for _ in 0..8 {
            crc = (crc >> 1) ^ if crc & 1 != 0 { 0xedb88320 } else { 0 };
        }
    }
    !crc
}

#[test]
fn megapixel_guard_reads_header_before_decoding_pixels() {
    let mut bytes = encode_image(&noise(1, 1, false), ImageFormat::Png).unwrap();
    for (width, height) in [
        (20_000u32, 20_000u32),
        (10_001, 10_000),
        (u32::MAX, u32::MAX),
    ] {
        bytes[16..20].copy_from_slice(&width.to_be_bytes());
        bytes[20..24].copy_from_slice(&height.to_be_bytes());
        let crc = crc32(&bytes[12..29]);
        bytes[29..33].copy_from_slice(&crc.to_be_bytes());
        // The pixel stream still describes 1×1. Decoding it would fail, but
        // the guard must report the header's pixel count first.
        let error = prepare_image(&bytes, &limits()).unwrap_err();
        assert!(
            matches!(error, ImageInputError::TooManyPixels { width: w, height: h, max: MAX_IMAGE_PIXELS } if w == width && h == height),
            "{error}"
        );
        assert!(error.to_string().contains("100 megapixels"));
    }
}

#[test]
fn corrupt_image_is_distinct_from_non_image_and_unsupported_format() {
    let bytes = encode_image(&noise(20, 20, false), ImageFormat::Png).unwrap();
    // Leave a valid IHDR, but truncate the pixel stream.
    assert!(matches!(
        prepare_image(&bytes[..40], &limits()),
        Err(ImageInputError::UnsupportedOrCorrupt { .. })
    ));
    assert!(matches!(
        prepare_image(b"\xff\xd8\xff\xe0", &limits()),
        Err(ImageInputError::UnsupportedOrCorrupt { .. })
    ));
    assert!(matches!(
        prepare_image(b"BMunsupported BMP", &limits()),
        Err(ImageInputError::UnsupportedOrCorrupt { .. })
    ));
    assert!(matches!(
        prepare_image(b"not an image", &limits()),
        Err(ImageInputError::NotAnImage)
    ));
}

#[test]
fn path_helper_uses_signature_and_reports_missing_files() {
    let directory = tempfile::tempdir().unwrap();
    let fake = directory.path().join("fake.png");
    std::fs::write(&fake, "ordinary text").unwrap();
    assert!(matches!(
        prepare_image_path(&fake, &limits()),
        Err(ImageInputError::NotAnImage)
    ));
    let actual = directory.path().join("actual.txt");
    let bytes = encode_image(&noise(8, 8, false), ImageFormat::Png).unwrap();
    std::fs::write(&actual, &bytes).unwrap();
    assert_eq!(
        payload(&prepare_image_path(&actual, &limits()).unwrap()),
        bytes
    );
    let missing = directory.path().join("missing.png");
    let error = prepare_image_path(&missing, &limits()).unwrap_err();
    assert!(matches!(&error, ImageInputError::Io { path, .. } if path == &missing));
    assert!(error.to_string().contains("missing.png"));
}

#[test]
fn impossible_limits_fail_instead_of_looping() {
    let bytes = encode_image(&noise(1, 1, true), ImageFormat::Png).unwrap();
    let mut limits = limits();
    limits.max_encoded_bytes = Some(EncodedSizeLimit::Bytes(1));
    assert!(matches!(
        prepare_image(&bytes, &limits),
        Err(ImageInputError::CannotFitLimits)
    ));
    limits.tile_budget = Some(0);
    assert!(matches!(
        prepare_image(&bytes, &limits),
        Err(ImageInputError::CannotFitLimits)
    ));
}
