//! Registries are values (PLAN.md D67): each `Runtime` owns its catalog of
//! custom providers and live models, and a closed session's children go.

mod agent_support;

use std::path::{Path, PathBuf};
use std::sync::Arc;

use agent_support::env;
use fast_harness_core::agent::{children, Agent, Conversation};
use fast_harness_core::app::{custom_models, Runtime, CACHE_FILE, CACHE_VERSION};
use fast_harness_core::config::{trust, Config};
use fast_harness_core::llm::anthropic::builtin_models;
use fast_harness_core::service::{Host, Service};
use serde_json::json;

fn write(path: &Path, text: &str) {
    std::fs::create_dir_all(path.parent().unwrap()).unwrap();
    std::fs::write(path, text).unwrap();
}

/// An `openai-chat` provider file `id` titled `title` with model `m1`.
fn provider(dir: &Path, id: &str, title: &str) {
    let file = json!({"id": id, "name": title, "api": "openai-chat",
        "baseUrl": "http://local/v1", "models": [{"id": "m1", "name": "M One"}]});
    write(&dir.join(format!("providers/{id}.json")), &file.to_string());
}

/// Ids, titles, owners and model rows a runtime's catalog shows.
fn sees(rt: &Runtime, id: &str) -> bool {
    let catalog = &rt.catalog;
    let model = format!("{id}/m1");
    let listed = catalog.providers().contains(&id);
    let owned = catalog.provider_of(&model) == id;
    let titled = catalog.group_title(id) != "Claude";
    let rows = custom_models(catalog).iter().any(|m| m.id == model);
    assert!(listed == owned && owned == titled && titled == rows, "{id}");
    listed
}

#[test]
fn two_runtimes_never_see_each_others_custom_providers() {
    let (a, b) = (env("catalog-a"), env("catalog-b"));
    provider(&a.home, "pa", "Provider A");
    provider(&b.home, "pb", "Provider B");
    let (ra, rb) = (
        Runtime::load_at(a.home.clone()),
        Runtime::load_at(b.home.clone()),
    );
    let (ra, rb) = (ra.unwrap(), rb.unwrap());
    assert!(sees(&ra, "pa") && !sees(&ra, "pb"));
    assert!(sees(&rb, "pb") && !sees(&rb, "pa"));
    assert_eq!(ra.catalog.group_title("pa"), "Provider A");
    assert_eq!(rb.catalog.display_name("pb/m1").as_deref(), Some("M One"));
    assert_eq!(ra.catalog.display_name("pb/m1"), None);
}

#[test]
fn a_projects_provider_leaves_with_the_project() {
    let e = env("catalog-project");
    let (project, other) = (e.cwd.join("p"), e.cwd.join("q"));
    std::fs::create_dir_all(&other).unwrap();
    provider(&project.join(".fast-harness"), "proj", "Project");
    trust::trust(&e.home, &project).unwrap();
    let mut rt = Runtime::load_at(e.home.clone()).unwrap();
    rt.use_project(&project).unwrap();
    let kept = rt.catalog.clone();
    assert!(sees(&rt, "proj"));
    rt.use_project(&other).unwrap();
    assert!(
        !sees(&rt, "proj"),
        "a second workspace sees the first's provider"
    );
    // An agent built for the first project keeps its providers.
    assert_eq!(kept.provider_of("proj/m1"), "proj");
}

#[test]
fn live_models_of_one_runtime_stay_there() {
    let (a, b) = (env("live-a"), env("live-b"));
    let mut info = serde_json::to_value(builtin_models()[0].clone()).unwrap();
    info["id"] = json!("claude-catalog-test");
    info["efforts"] = json!(["low", "high"]);
    info["live"] = json!({"effort": true, "adaptive": true, "budget": false, "canDisable": false});
    let cache = json!({"version": CACHE_VERSION, "fetchedAt": 1, "models": [info]});
    write(&a.home.join(CACHE_FILE), &cache.to_string());
    let ra = Runtime::load_at(a.home.clone()).unwrap();
    let rb = Runtime::load_at(b.home.clone()).unwrap();
    assert_eq!(ra.catalog.efforts("claude-catalog-test"), ["low", "high"]);
    let all = ["low", "medium", "high", "xhigh", "max", "off"];
    assert_eq!(rb.catalog.efforts("claude-catalog-test"), all);
    // The runtime's Claude provider reads the same table.
    assert!(Arc::ptr_eq(ra.provider.live(), ra.catalog.live()));
}

/// A host whose sessions never run a turn.
struct Idle(PathBuf);

impl Host for Idle {
    fn home(&self) -> PathBuf {
        self.0.clone()
    }

    fn config(&self, _: &Path) -> Result<Config, String> {
        Ok(Config::default())
    }

    fn agent(
        &self,
        _: &Path,
        _: &Conversation,
        _: &Config,
    ) -> Result<(Agent, Vec<String>), String> {
        Err("no turns here".into())
    }
}

#[test]
fn a_closed_sessions_children_are_gone() {
    let e = env("children-close");
    let service = Service::new(Arc::new(Idle(e.home.clone())));
    let session = service.create(&e.cwd).unwrap();
    let id = session.id.clone();
    let _ = children::of_session(&id);
    assert!(children::is_known(&id));
    service.close(&id);
    assert!(
        !children::is_known(&id),
        "the closed session's registry stays"
    );
}

/// Every Rust source file under `dir`.
fn sources(dir: &Path, out: &mut Vec<PathBuf>) {
    for entry in std::fs::read_dir(dir).unwrap().flatten() {
        let path = entry.path();
        if path.is_dir() {
            sources(&path, out);
        } else if path.extension().is_some_and(|ext| ext == "rs") {
            out.push(path);
        }
    }
}

#[test]
fn no_leaked_names_or_process_wide_model_registries() {
    let src = Path::new(env!("CARGO_MANIFEST_DIR")).join("src");
    let mut files = Vec::new();
    sources(&src, &mut files);
    let (leak, registry) = (["Box", "::leak"].concat(), ["static ", "REGISTRY"].concat());
    for file in files {
        let text = std::fs::read_to_string(&file).unwrap();
        assert!(!text.contains(&leak), "{}", file.display());
        assert!(!text.contains(&registry), "{}", file.display());
    }
}
