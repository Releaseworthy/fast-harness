use serde_json::json;

use super::*;

fn no_env(_: &str) -> Option<String> {
    None
}

#[test]
fn the_default_renders_at_scale_one_on_the_vsync_renderer() {
    // Before D75 nothing set the scale: the window followed the Windows
    // scale (2.0 on a 200% monitor, a 2200x1480 client for 1100x740).
    let plan = plan(&serde_json::Value::Null, &no_env);
    assert_eq!(plan, Plan { renderer: None, scale: Some(1.0), select: true });
}

#[test]
fn ui_scale_is_clamped_and_a_preset_env_scale_wins() {
    let scale = |v: serde_json::Value| plan(&json!({ "uiScale": v }), &no_env).scale;
    assert_eq!(scale(json!(1.25)), Some(1.25));
    assert_eq!(scale(json!(9)), Some(4.0));
    assert_eq!(scale(json!(0.1)), Some(0.5));
    assert_eq!(scale(json!("big")), Some(1.0));
    let env = |k: &str| (k == "SLINT_SCALE_FACTOR").then(|| "2".to_string());
    assert_eq!(plan(&json!({ "uiScale": 1.5 }), &env).scale, None);
}

#[test]
fn renderer_names_map_to_winit_renderers() {
    let renderer = |v: &str| plan(&json!({ "renderer": v }), &no_env).renderer;
    assert_eq!(renderer("software"), Some("software"));
    assert_eq!(renderer("skia"), Some("skia"));
    assert_eq!(renderer("d3d"), Some("skia"));
    assert_eq!(renderer("directx"), Some("skia"));
    // auto leaves Slint's own order: Skia (D3D12, then its CPU surface),
    // then software. OpenGL is gone (owner, 2026-11-14): its old names
    // and unknown ones read as auto.
    assert_eq!(renderer("auto"), None);
    assert_eq!(renderer("femtovg"), None);
    assert_eq!(renderer("gl"), None);
    assert_eq!(renderer("vulkan"), None);
    let env = |k: &str| (k == "SLINT_BACKEND").then(|| "winit-software".to_string());
    assert!(!plan(&serde_json::Value::Null, &env).select);
}
