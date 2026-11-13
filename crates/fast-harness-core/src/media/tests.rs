use super::*;

fn temp(name: &str) -> std::path::PathBuf {
    let dir = std::env::temp_dir().join(format!("fh-media-{name}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    dir
}

#[test]
fn attach_keeps_one_hashed_file_and_reads_it_back() {
    let dir = temp("attach");
    let png = solid_png(4, 4, [0, 128, 255]);
    let one = attach_bytes(&dir, &png, Some("image/png")).unwrap();
    let two = attach_bytes(&dir, &png, None).unwrap();
    assert_eq!(one, two);
    let name = Path::new(&one.path)
        .file_name()
        .unwrap()
        .to_string_lossy()
        .to_string();
    assert_eq!(name, format!("{}.png", &hex(&sha256(&png))[..32]));
    assert_eq!((one.width, one.height, one.bytes), (4, 4, png.len() as u64));
    assert_eq!(base64(&one).unwrap(), base64_encode(&png));
    let file = dir.join("a.png");
    std::fs::write(&file, &png).unwrap();
    assert_eq!(attach_file(&dir, &file).unwrap(), one);
    std::fs::remove_file(&one.path).unwrap();
    assert!(base64(&one).unwrap_err().contains("cannot be read"));
    std::fs::remove_dir_all(&dir).unwrap();
}

#[test]
fn types_sizes_and_edges_are_refused_with_words() {
    let png = solid_png(2, 2, [1, 2, 3]);
    let refused = |bytes: &[u8], kind: Option<&str>| check(bytes, kind).unwrap_err();
    assert_eq!(
        refused(&png, Some("image/bmp")),
        MediaError::Unsupported("image/bmp".into())
    );
    assert_eq!(refused(b"BM not an image", None), MediaError::NotImage);
    assert!(matches!(
        refused(&png, Some("image/jpeg")),
        MediaError::Mismatch { .. }
    ));
    assert_eq!(
        check(&png, Some("IMAGE/PNG")).unwrap(),
        ("image/png", (2, 2))
    );
    let mut big = png.clone();
    big.resize(MAX_BYTES as usize + 1, 0);
    let too_large = refused(&big, None);
    assert!(
        too_large.to_string().contains("the cap is 3.6 MB"),
        "{too_large}"
    );
    let wide = solid_png(MAX_EDGE + 1, 1, [0; 3]);
    let words = refused(&wide, None).to_string();
    assert_eq!(
        words,
        "the image is 8001x1 px and the longest edge allowed is 8000 px"
    );
    assert_eq!(refused(b"GIF89a\0\0\0\0", None), MediaError::NoSize);
    let dir = temp("cap");
    let file = dir.join("big.png");
    std::fs::create_dir_all(&dir).unwrap();
    std::fs::write(&file, &big).unwrap();
    assert!(matches!(
        attach_file(&dir, &file),
        Err(MediaError::TooLarge { .. })
    ));
    std::fs::remove_dir_all(&dir).unwrap();
}

#[test]
fn token_estimates_follow_the_published_table() {
    let at = |w, h| tokens(&ImageRef::new("x", "image/png", 1, (w, h)));
    // platform.claude.com vision, high-resolution tier (2026-11-13).
    assert_eq!(at(200, 200), 64);
    assert_eq!(at(1000, 1000), 1296);
    assert_eq!(at(1920, 1080), 2691);
    assert_eq!(at(3840, 2160), 4784);
    let image = ImageRef::new("x", "image/png", 12_600, (640, 480));
    assert_eq!(label(&image), "png 640x480, 12.3 KB, about 414 tokens");
    assert!(placeholder(&image, "no").starts_with("[image not sent: no; png 640x480"));
}
