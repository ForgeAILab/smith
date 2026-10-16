//! Bounded header descriptions for image data URLs.

use base64::Engine;
use base64::engine::general_purpose::STANDARD;

/// Media type and optional pixel dimensions, without the image's data.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ImageHeader {
    /// The data URL's media type, or `image` for an unknown or unsafe header.
    pub mime: String,
    /// Pixel width and height when a supported header can be decoded.
    pub dimensions: Option<(u32, u32)>,
}

/// Describes an image without fetching its URL or decoding its full payload.
///
/// PNG, GIF, and WebP headers use at most 4 KiB of decoded data. JPEG frame
/// headers can follow metadata, so those use at most 64 KiB. Malformed or
/// unsupported headers keep the media type with no dimensions; non-data URLs
/// use `image`.
pub fn describe_image_url(url: &str) -> ImageHeader {
    let Some((mime, data)) = url
        .strip_prefix("data:")
        .and_then(|rest| rest.split_once(";base64,"))
    else {
        return ImageHeader {
            mime: "image".to_owned(),
            dimensions: None,
        };
    };
    // A malformed header must not smuggle controls or an unbounded value into
    // the otherwise data-free description.
    let mime = if !mime.is_empty()
        && mime.len() <= 128
        && mime
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || b"/.-+".contains(&byte))
    {
        mime
    } else {
        "image"
    };
    ImageHeader {
        mime: mime.to_owned(),
        dimensions: decoded_dimensions(data),
    }
}

fn decoded_dimensions(data: &str) -> Option<(u32, u32)> {
    let prefix = decode_prefix(data, 4 * 1024)?;
    if let Some(dimensions) = image_dimensions(&prefix) {
        return Some(dimensions);
    }
    // JPEG metadata can precede the frame header. Never decode more than
    // 64 KiB, even for a multi-megabyte image or a malformed result.
    if prefix.starts_with(&[0xff, 0xd8]) && data.len() > prefix.len().div_ceil(3) * 4 {
        return image_dimensions(&decode_prefix(data, 64 * 1024)?);
    }
    None
}

fn decode_prefix(data: &str, max_bytes: usize) -> Option<Vec<u8>> {
    let encoded_limit = max_bytes / 3 * 4;
    STANDARD
        .decode(&data.as_bytes()[..data.len().min(encoded_limit)])
        .ok()
}

pub(crate) fn image_dimensions(bytes: &[u8]) -> Option<(u32, u32)> {
    let dimensions = if bytes.starts_with(b"\x89PNG\r\n\x1a\n") {
        if bytes.get(8..16)? != b"\0\0\0\rIHDR" {
            return None;
        }
        (be_u32(bytes.get(16..20)?)?, be_u32(bytes.get(20..24)?)?)
    } else if bytes.starts_with(b"GIF87a") || bytes.starts_with(b"GIF89a") {
        (le_u16(bytes.get(6..8)?)?, le_u16(bytes.get(8..10)?)?)
    } else if bytes.starts_with(&[0xff, 0xd8]) {
        jpeg_dimensions(bytes)?
    } else if bytes.get(..4)? == b"RIFF" && bytes.get(8..12)? == b"WEBP" {
        match bytes.get(12..16)? {
            b"VP8X" => (
                le_u24(bytes.get(24..27)?)? + 1,
                le_u24(bytes.get(27..30)?)? + 1,
            ),
            b"VP8L" if bytes.get(20) == Some(&0x2f) => {
                let packed = u32::from_le_bytes(bytes.get(21..25)?.try_into().ok()?);
                ((packed & 0x3fff) + 1, ((packed >> 14) & 0x3fff) + 1)
            }
            b"VP8 " if bytes.get(23..26)? == b"\x9d\x01\x2a" => (
                le_u16(bytes.get(26..28)?)? & 0x3fff,
                le_u16(bytes.get(28..30)?)? & 0x3fff,
            ),
            _ => return None,
        }
    } else {
        return None;
    };
    (dimensions.0 > 0 && dimensions.1 > 0).then_some(dimensions)
}

fn jpeg_dimensions(bytes: &[u8]) -> Option<(u32, u32)> {
    let mut offset = 2;
    loop {
        if *bytes.get(offset)? != 0xff {
            return None;
        }
        while bytes.get(offset) == Some(&0xff) {
            offset += 1;
        }
        let marker = *bytes.get(offset)?;
        offset += 1;
        match marker {
            0x00 | 0xd9 | 0xda => return None,
            0x01 | 0xd0..=0xd8 => continue,
            _ => {}
        }
        let length = usize::from(u16::from_be_bytes(
            bytes.get(offset..offset + 2)?.try_into().ok()?,
        ));
        if length < 2 {
            return None;
        }
        if matches!(marker, 0xc0..=0xc3 | 0xc5..=0xc7 | 0xc9..=0xcb | 0xcd..=0xcf) {
            if length < 8 {
                return None;
            }
            let height = u16::from_be_bytes(bytes.get(offset + 3..offset + 5)?.try_into().ok()?);
            let width = u16::from_be_bytes(bytes.get(offset + 5..offset + 7)?.try_into().ok()?);
            return Some((u32::from(width), u32::from(height)));
        }
        offset += length;
    }
}

fn be_u32(bytes: &[u8]) -> Option<u32> {
    Some(u32::from_be_bytes(bytes.try_into().ok()?))
}

fn le_u16(bytes: &[u8]) -> Option<u32> {
    Some(u32::from(u16::from_le_bytes(bytes.try_into().ok()?)))
}

fn le_u24(bytes: &[u8]) -> Option<u32> {
    Some(u32::from_le_bytes([
        *bytes.first()?,
        *bytes.get(1)?,
        *bytes.get(2)?,
        0,
    ]))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn image_dimensions_cover_jpeg_sof_variants_and_webp_headers() {
        for marker in [
            0xc0, 0xc1, 0xc2, 0xc3, 0xc5, 0xc6, 0xc7, 0xc9, 0xca, 0xcb, 0xcd, 0xce, 0xcf,
        ] {
            let jpeg = [0xff, 0xd8, 0xff, marker, 0, 8, 8, 0, 3, 0, 2, 1];
            assert_eq!(image_dimensions(&jpeg), Some((2, 3)));
        }
        let mut extended = b"RIFF\x16\0\0\0WEBPVP8X\x0a\0\0\0\0\0\0\0".to_vec();
        extended.extend([1, 0, 0, 2, 0, 0]);
        assert_eq!(image_dimensions(&extended), Some((2, 3)));

        let mut lossless = b"RIFF\x11\0\0\0WEBPVP8L\x05\0\0\0\x2f".to_vec();
        lossless.extend((1u32 | (2 << 14)).to_le_bytes());
        assert_eq!(image_dimensions(&lossless), Some((2, 3)));

        let mut lossy = b"RIFF\x16\0\0\0WEBPVP8 \x0a\0\0\0\0\0\0\x9d\x01\x2a".to_vec();
        lossy.extend([2, 0, 3, 0]);
        assert_eq!(image_dimensions(&lossy), Some((2, 3)));
        assert_eq!(image_dimensions(b"GIF87a\x02\0\x03\0"), Some((2, 3)));
        assert_eq!(image_dimensions(b"GIF89a\0\0\x03\0"), None);
    }

    #[test]
    fn image_prefix_decoding_stops_at_64_kib_and_tolerates_unread_image_tails() {
        let mut jpeg = vec![0xff, 0xd8, 0xff, 0xe1];
        let segment_length = 5000u16;
        jpeg.extend(segment_length.to_be_bytes());
        jpeg.resize(4 + usize::from(segment_length), 0);
        jpeg.extend([0xff, 0xc2, 0, 8, 8, 0, 3, 0, 2, 1]);
        assert_eq!(decoded_dimensions(&STANDARD.encode(&jpeg)), Some((2, 3)));

        let mut past_cap = vec![0xff, 0xd8, 0xff, 0xe1, 0xff, 0xff];
        past_cap.resize(4 + usize::from(u16::MAX), 0);
        past_cap.extend([0xff, 0xc0, 0, 8, 8, 0, 3, 0, 2, 1]);
        assert_eq!(decoded_dimensions(&STANDARD.encode(&past_cap)), None);

        let encoded = STANDARD.encode(vec![0u8; 100_000]);
        assert!(decode_prefix(&encoded, 64 * 1024).unwrap().len() <= 64 * 1024);
        let mut gif = b"GIF89a\x02\0\x03\0".to_vec();
        gif.resize(5000, 0);
        let encoded = format!("{}!unread tail", STANDARD.encode(&gif));
        assert_eq!(decoded_dimensions(&encoded), Some((2, 3)));
        for data in ["", "!", "AAAAA", "8J+YgA=="] {
            assert_eq!(decoded_dimensions(data), None);
        }
        assert_eq!(image_dimensions(&[0xff, 0xd8, 0xff, 0xe1, 0, 1]), None);
    }

    #[test]
    fn image_headers_describe_png_and_unknown_dimensions_without_urls() {
        let mut png = b"\x89PNG\r\n\x1a\n\0\0\0\rIHDR".to_vec();
        png.extend(1440u32.to_be_bytes());
        png.extend(900u32.to_be_bytes());
        let url = format!("data:image/png;base64,{}", STANDARD.encode(&png));
        assert_eq!(
            describe_image_url(&url),
            ImageHeader {
                mime: "image/png".to_owned(),
                dimensions: Some((1440, 900)),
            }
        );
        for (url, mime) in [
            ("https://example.com/secret-image.png", "image"),
            ("data:image/jpeg;base64,invalid!", "image/jpeg"),
            ("data:image/png;base64,", "image/png"),
            ("data:image/png\nPRIVATE;base64,AAAA", "image"),
        ] {
            assert_eq!(
                describe_image_url(url),
                ImageHeader {
                    mime: mime.to_owned(),
                    dimensions: None,
                }
            );
        }
        png[16..20].copy_from_slice(&0u32.to_be_bytes());
        assert_eq!(image_dimensions(&png), None);
        assert_eq!(image_dimensions(b"\x89PNG\r\n\x1a\n"), None);
    }
}
