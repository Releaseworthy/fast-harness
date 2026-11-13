use super::*;
use crate::media::solid_png;

#[test]
fn png_gif_webp_and_jpeg_sizes_read_from_headers() {
    let png = solid_png(3, 2, [255, 0, 0]);
    assert_eq!(
        (media_type(&png), dimensions(&png)),
        (Some("image/png"), Some((3, 2)))
    );
    let gif = b"GIF89a\x40\x01\xf0\x00\x00\x00\x00;".to_vec();
    assert_eq!(
        (media_type(&gif), dimensions(&gif)),
        (Some("image/gif"), Some((320, 240)))
    );
    let mut webp = b"RIFF\0\0\0\0WEBPVP8X\x0a\0\0\0\0\0\0\0".to_vec();
    webp.extend_from_slice(&[0x7F, 0x02, 0x00, 0xDF, 0x01, 0x00]); // 640 x 480
    assert_eq!(
        (media_type(&webp), dimensions(&webp)),
        (Some("image/webp"), Some((640, 480)))
    );
    let mut lossless = b"RIFF\0\0\0\0WEBPVP8L\0\0\0\0\x2f".to_vec();
    let bits: u32 = 99 | (49 << 14); // 100 x 50
    lossless.extend_from_slice(&bits.to_le_bytes());
    assert_eq!(dimensions(&lossless), Some((100, 50)));
    // SOI, an APP0 segment, then SOF0 with height 0x01E0 and width 0x0280.
    let jpeg = [
        0xFF, 0xD8, 0xFF, 0xE0, 0x00, 0x04, 0x4A, 0x46, 0xFF, 0xC0, 0x00, 0x11, 0x08, 0x01, 0xE0,
        0x02, 0x80, 0x03,
    ];
    assert_eq!(
        (media_type(&jpeg), dimensions(&jpeg)),
        (Some("image/jpeg"), Some((640, 480)))
    );
}

#[test]
fn other_and_cut_bytes_read_as_nothing() {
    assert_eq!(media_type(b"BM\0\0\0\0"), None);
    assert_eq!(dimensions(b"hello"), None);
    assert_eq!(dimensions(&solid_png(3, 2, [0; 3])[..18]), None);
    assert_eq!(dimensions(&[0xFF, 0xD8, 0xFF, 0xE0, 0x00]), None);
    assert_eq!(
        dimensions(b"GIF89a\0\0\0\0"),
        None,
        "a zero size is no size"
    );
}
