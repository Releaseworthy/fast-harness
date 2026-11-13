// Fast Harness patch (docs/decisions/D78.md): subpixel (LCD, ClearType-style)
// text. `SLINT_SKIA_TEXT_AA` = `rgb` or `bgr` gives surfaces that pixel
// geometry and glyphs subpixel edging; anything else keeps Slint's grayscale.
// Read once, before the first surface or glyph.

use std::sync::OnceLock;

fn geometry() -> skia_safe::PixelGeometry {
    static GEOMETRY: OnceLock<skia_safe::PixelGeometry> = OnceLock::new();
    *GEOMETRY.get_or_init(|| match std::env::var("SLINT_SKIA_TEXT_AA").as_deref() {
        Ok("rgb") => skia_safe::PixelGeometry::RGBH,
        Ok("bgr") => skia_safe::PixelGeometry::BGRH,
        _ => skia_safe::PixelGeometry::Unknown,
    })
}

/// The surface properties every window surface is created with.
pub fn surface_props() -> skia_safe::SurfaceProps {
    skia_safe::SurfaceProps::new(skia_safe::SurfacePropsFlags::default(), geometry())
}

/// The glyph edging: subpixel when the surfaces have an LCD geometry.
pub fn edging() -> skia_safe::font::Edging {
    match geometry() {
        skia_safe::PixelGeometry::Unknown => skia_safe::font::Edging::AntiAlias,
        _ => skia_safe::font::Edging::SubpixelAntiAlias,
    }
}