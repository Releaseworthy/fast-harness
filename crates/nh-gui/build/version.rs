//! The VS_VERSIONINFO resource: a fixed file info plus FileDescription,
//! ProductName and the version strings, as nested length-prefixed blocks.

fn utf16z(text: &str) -> Vec<u8> {
    text.encode_utf16()
        .chain([0])
        .flat_map(u16::to_le_bytes)
        .collect()
}

fn pad4(out: &mut Vec<u8>) {
    while !out.len().is_multiple_of(4) {
        out.push(0);
    }
}

/// One block: wLength, wValueLength, wType, the key, the value, children.
/// `value_len` counts words for text values and bytes for binary ones.
fn block(key: &str, value: &[u8], value_len: usize, text: bool, children: &[Vec<u8>]) -> Vec<u8> {
    let mut out = vec![0, 0];
    let value_len = u16::try_from(value_len).unwrap_or(0);
    out.extend_from_slice(&value_len.to_le_bytes());
    out.extend_from_slice(&u16::from(text).to_le_bytes());
    out.extend_from_slice(&utf16z(key));
    pad4(&mut out);
    out.extend_from_slice(value);
    for child in children {
        pad4(&mut out);
        out.extend_from_slice(child);
    }
    let len = u16::try_from(out.len()).unwrap_or(u16::MAX);
    out[..2].copy_from_slice(&len.to_le_bytes());
    out
}

fn string(key: &str, value: &str) -> Vec<u8> {
    let words = value.encode_utf16().count() + 1;
    block(key, &utf16z(value), words, true, &[])
}

/// `1.2.3` as the two DWORDs of a VS_FIXEDFILEINFO version.
fn version_words(version: &str) -> [u32; 2] {
    let mut parts = version
        .split(['.', '-', '+'])
        .map(|p| p.parse::<u32>().unwrap_or(0) & 0xFFFF);
    let mut next = || parts.next().unwrap_or(0);
    let (a, b, c, d) = (next(), next(), next(), next());
    [(a << 16) | b, (c << 16) | d]
}

fn fixed(version: &str) -> Vec<u8> {
    let [ms, ls] = version_words(version);
    // Signature, struct version, file and product versions, flags mask,
    // flags, OS (VOS_NT_WINDOWS32), type (VFT_APP), subtype, date.
    let words = [
        0xFEEF_04BD,
        0x0001_0000,
        ms,
        ls,
        ms,
        ls,
        0x3F,
        0,
        0x0004_0004,
        1,
        0,
        0,
        0,
    ];
    words.iter().flat_map(|w| w.to_le_bytes()).collect()
}

/// The VS_VERSIONINFO bytes for `version` with `name` as the product.
pub fn info(version: &str, name: &str) -> Vec<u8> {
    let strings = [
        string("FileDescription", name),
        string("ProductName", name),
        string("FileVersion", version),
        string("ProductVersion", version),
        string("OriginalFilename", "nh-gui.exe"),
    ];
    let table = block("040904B0", &[], 0, true, &strings);
    let string_info = block("StringFileInfo", &[], 0, true, &[table]);
    let translation = [0x09, 0x04, 0xB0, 0x04];
    let var = block("Translation", &translation, 4, false, &[]);
    let var_info = block("VarFileInfo", &[], 0, true, &[var]);
    let fixed = fixed(version);
    block(
        "VS_VERSION_INFO",
        &fixed,
        fixed.len(),
        false,
        &[string_info, var_info],
    )
}
