//! A Win32 `.res` file built from an `.ico`: RT_GROUP_ICON 1, one RT_ICON
//! per ICO image and a VERSIONINFO, in the format link.exe takes as input.
//! Pure Rust, used only by build.rs.

const RT_ICON: u16 = 3;
const RT_GROUP_ICON: u16 = 14;
const RT_VERSION: u16 = 16;
/// English (United States).
const LANG: u16 = 0x0409;
/// MOVEABLE | PURE (| DISCARDABLE for icons), as rc.exe writes them.
const FLAGS_ICON: u16 = 0x1010;
const FLAGS_GROUP: u16 = 0x1030;

/// One image of an ICO file: its 16-byte directory entry minus the offset.
pub struct IconImage<'a> {
    /// Width, height, colour count, reserved, planes (2), bit count (2).
    head: [u8; 8],
    data: &'a [u8],
}

fn u16_at(bytes: &[u8], at: usize) -> Option<u16> {
    Some(u16::from_le_bytes(bytes.get(at..at + 2)?.try_into().ok()?))
}

fn u32_at(bytes: &[u8], at: usize) -> Option<usize> {
    let raw = u32::from_le_bytes(bytes.get(at..at + 4)?.try_into().ok()?);
    usize::try_from(raw).ok()
}

/// The images of an ICO file; `None` when it is not a well-formed icon.
pub fn parse_ico(ico: &[u8]) -> Option<Vec<IconImage<'_>>> {
    if u16_at(ico, 0)? != 0 || u16_at(ico, 2)? != 1 {
        return None;
    }
    let count = usize::from(u16_at(ico, 4)?);
    let mut images = Vec::with_capacity(count);
    for i in 0..count {
        let entry = 6 + i * 16;
        let head: [u8; 8] = ico.get(entry..entry + 8)?.try_into().ok()?;
        let size = u32_at(ico, entry + 8)?;
        let offset = u32_at(ico, entry + 12)?;
        let data = ico.get(offset..offset.checked_add(size)?)?;
        images.push(IconImage { head, data });
    }
    (!images.is_empty()).then_some(images)
}

fn pad4(out: &mut Vec<u8>) {
    while !out.len().is_multiple_of(4) {
        out.push(0);
    }
}

/// One resource: a 32-byte RESOURCEHEADER with a numeric type and name,
/// then the data, padded to a DWORD.
fn entry(out: &mut Vec<u8>, kind: u16, name: u16, flags: u16, data: &[u8]) {
    let size = u32::try_from(data.len()).unwrap_or(u32::MAX);
    out.extend_from_slice(&size.to_le_bytes());
    out.extend_from_slice(&32u32.to_le_bytes());
    for id in [kind, name] {
        out.extend_from_slice(&0xFFFFu16.to_le_bytes());
        out.extend_from_slice(&id.to_le_bytes());
    }
    out.extend_from_slice(&0u32.to_le_bytes()); // DataVersion
    out.extend_from_slice(&flags.to_le_bytes());
    out.extend_from_slice(&LANG.to_le_bytes());
    out.extend_from_slice(&[0; 8]); // Version, Characteristics
    out.extend_from_slice(data);
    pad4(out);
}

/// GRPICONDIR: the ICO directory with resource ids instead of offsets.
fn group(images: &[IconImage<'_>]) -> Vec<u8> {
    let mut out = vec![0, 0, 1, 0];
    let count = u16::try_from(images.len()).unwrap_or(u16::MAX);
    out.extend_from_slice(&count.to_le_bytes());
    for (i, image) in images.iter().enumerate() {
        out.extend_from_slice(&image.head);
        let size = u32::try_from(image.data.len()).unwrap_or(u32::MAX);
        out.extend_from_slice(&size.to_le_bytes());
        let id = u16::try_from(i + 1).unwrap_or(u16::MAX);
        out.extend_from_slice(&id.to_le_bytes());
    }
    out
}

/// The whole `.res` file: the empty entry, the icons, the group, the version.
pub fn build(ico: &[u8], version: &str, name: &str) -> Option<Vec<u8>> {
    let images = parse_ico(ico)?;
    let mut out = Vec::new();
    entry(&mut out, 0, 0, 0, &[]);
    for (i, image) in images.iter().enumerate() {
        let id = u16::try_from(i + 1).ok()?;
        entry(&mut out, RT_ICON, id, FLAGS_ICON, image.data);
    }
    entry(&mut out, RT_GROUP_ICON, 1, FLAGS_GROUP, &group(&images));
    let info = crate::version::info(version, name);
    entry(&mut out, RT_VERSION, 1, FLAGS_GROUP, &info);
    Some(out)
}
