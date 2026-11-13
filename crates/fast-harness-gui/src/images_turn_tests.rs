//! An image attached in the window goes with the next prompt (PLAN.md D70
//! "Views"): the body carries it, the user row has its thumbnail chip and
//! label, and the live item equals the replay of the session log.

use fast_harness_core::media;
use fast_harness_core::view::Transcript;
use slint::Model;

use super::*;
use crate::rows::msg_row;

#[test]
fn an_attached_image_goes_with_the_prompt_and_the_row_equals_the_replay() {
    let mut w = Window::new("image");
    let id = w.show_new();
    let png = w.dir.join("dot.png");
    std::fs::write(&png, media::solid_png(3, 2, [1, 2, 3])).unwrap();
    let image = w.session(&id).attach_image_file(&png).unwrap();
    let chips = crate::images_view::chips(std::slice::from_ref(&image));
    assert!(chips.row_data(0).unwrap().label.starts_with("png 3x2, "));
    let script = Arc::new(ScriptedTransport::new(vec![text_reply("seen")]));
    let parts = AgentParts {
        catalog: Default::default(),
        extensions: Default::default(),
        provider: Arc::new(Anthropic::new("http://scripted.invalid")),
        transport: script.clone(),
        auth: Arc::new(StaticAuth::new("acct-1", "test", Auth::ApiKey("k".into()))),
        config: Default::default(),
        cwd: w.dir.clone(),
        system: "test system".into(),
    };
    let agent = Agent::new(parts, Arc::new(Registry::new()));
    w.start_agent(&id, Start::Text("what is it?".into()), agent).unwrap();
    w.pump(&id);
    let body: Value = serde_json::from_str(&script.requests()[0].body).unwrap();
    let user = (body["messages"].as_array().unwrap().iter()).rfind(|m| m["role"] == "user");
    let content = user.unwrap()["content"].as_array().unwrap().clone();
    assert!(content.iter().any(|b| b["type"] == "image"), "{content:?}");
    assert!(w.session(&id).attached_images().is_empty(), "the turn took it");
    let live = (w.runs.shown.view.items.iter())
        .find(|i| matches!(i, Item::User { .. }))
        .cloned()
        .unwrap();
    let Item::User { text, images } = &live else { unreachable!() };
    assert_eq!(text, &media::user_text("what is it?", std::slice::from_ref(&image)));
    assert_eq!(images, &vec![image]);
    let row = msg_row(&live);
    assert_eq!(row.images.row_count(), 1);
    assert!(row.text.starts_with("[image 1: png 3x2, "), "{}", row.text);
    let rt = fast_harness_core::app::Runtime::load_at(w.dir.clone()).unwrap();
    let (conv, events) = rt.open_with_events(&w.dir, Some(&id), false).unwrap();
    let replay = Transcript::opened(&events, &conv.messages, conv.take_notes());
    let first = replay.items.iter().find(|i| matches!(i, Item::User { .. }));
    assert_eq!(first, Some(&live));
}