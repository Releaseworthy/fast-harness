//! The settings layers and `providers/*.json` wired into [`Runtime`]
//! (PLAN.md D41): loading both, the routes of custom providers, project
//! trust and the one-time notes.

use std::path::Path;
use std::sync::Arc;

use super::{Route, Runtime};
use crate::auth::{AuthSource, StaticAuth};
use crate::config::index::{ExtIndex, Item};
use crate::config::layers::{self, kind_files, Layered, Layers};
use crate::config::{trust, Config};
use crate::http::Transport;
use crate::llm::anthropic::Anthropic;
use crate::llm::custom::{self, ProviderFile, API_OPENAI};
use crate::llm::openai::OpenAiChat;
use crate::llm::{Auth, ChatRequest, Completion, Delta, LlmError, Provider};
use crate::tools::hooks::Hooks;
use crate::tools::Extensions;

/// The resolved layers and the provider files they found.
pub struct Settings {
    pub layered: Layered,
    pub providers: Vec<ProviderFile>,
    /// The tool, agent and command files of the same folders (D42).
    pub index: ExtIndex,
    /// `hooks.json` of the same folders (D44); empty without a file.
    pub hooks: Hooks,
}

/// Every layer for `cwd` (the user layer only without one), then the
/// `providers/*.json` files of the same folders. Only a bad user
/// `config.json` is an error; other bad files are skipped in the report.
pub fn load_settings(
    home: &Path,
    cwd: Option<&Path>,
    env: &dyn Fn(&str) -> Option<String>,
) -> Result<Settings, String> {
    let found = match cwd {
        Some(cwd) => Layers::find(home, cwd),
        None => Layers::user_only(home),
    };
    let mut layered = layers::load(found, env)?;
    let files = kind_files(&layered.layers, "providers", "json");
    let (providers, loaded, skipped) = custom::read(&files);
    layered.report.loaded.extend(loaded);
    layered.report.skipped.extend(skipped);
    add_compaction(&mut layered.config, &providers);
    let index = ExtIndex::build(&layered.layers);
    layered.report.skipped.extend(index.skipped.iter().cloned());
    let (hooks, skipped) = Hooks::load(&layered.layers);
    layered.report.loaded.extend(hooks.files.iter().cloned());
    layered.report.skipped.extend(skipped);
    Ok(Settings {
        layered,
        providers,
        index,
        hooks,
    })
}

/// A model entry's `compaction` joins `models` under its full id unless
/// `config.json` has an entry for that exact id (which then wins).
fn add_compaction(config: &mut Config, providers: &[ProviderFile]) {
    for provider in providers {
        for model in &provider.models {
            if model.compaction == Default::default() {
                continue;
            }
            let id = match provider.id.as_str() {
                "anthropic" => model.id.clone(),
                prefix => format!("{prefix}/{}", model.id),
            };
            let entry = config.models.entry(id).or_default();
            if entry.compaction == Default::default() {
                entry.compaction = model.compaction.clone();
            }
        }
    }
}

/// An `anthropic-messages` provider of a file: Claude's adapter at the
/// file's `baseUrl` with the model id after the `<id>/` prefix.
struct Renamed {
    id: String,
    inner: Anthropic,
}

impl Provider for Renamed {
    fn id(&self) -> &str {
        &self.id
    }

    fn stream(
        &self,
        http: &dyn Transport,
        auth: &Auth,
        req: &ChatRequest<'_>,
        on_delta: &mut dyn FnMut(Delta<'_>),
    ) -> Result<Completion, LlmError> {
        let model = match req.model.split_once('/') {
            Some((prefix, rest)) if prefix == self.id => rest,
            _ => req.model,
        };
        let renamed = ChatRequest { model, ..*req };
        self.inner.stream(http, auth, &renamed, on_delta)
    }
}

/// The adapter of a custom provider file.
pub fn provider_for(file: &ProviderFile) -> Arc<dyn Provider> {
    if file.api.as_deref() == Some(API_OPENAI) {
        let headers = file.headers.clone().into_iter().collect();
        return Arc::new(OpenAiChat::new(&file.id, &file.base_url, headers));
    }
    Arc::new(Renamed {
        id: file.id.clone(),
        inner: Anthropic::new(&file.base_url),
    })
}

fn env_value(key: &str) -> Option<String> {
    let value = std::env::var(key).ok()?;
    let value = value.trim();
    (!value.is_empty()).then(|| value.to_string())
}

impl Runtime {
    /// Re-reads the layers for `cwd` (its project folder when trusted) and
    /// the provider files; an untrusted project queues its note once.
    pub fn use_project(&mut self, cwd: &Path) -> Result<(), String> {
        let settings = load_settings(&self.home, Some(cwd), &env_value)?;
        self.apply(settings);
        Ok(())
    }

    /// Trusts the project of `cwd` (else `cwd` itself) and reloads it.
    pub fn trust_project(&mut self, cwd: &Path) -> Result<String, String> {
        let root = layers::project_root(&self.home, cwd).unwrap_or_else(|| cwd.to_path_buf());
        let path = trust::trust(&self.home, &root)?;
        self.use_project(cwd)?;
        Ok(format!(
            "trusted {}: its .fast-harness settings now load (a new session picks up a changed system prompt)",
            path.display()
        ))
    }

    pub(super) fn apply(&mut self, settings: Settings) {
        let Settings {
            mut layered,
            providers,
            index,
            hooks,
        } = settings;
        let extensions = Extensions::new(index).with_hooks(hooks);
        // A tool file's `outputBytes` unless `toolOutput` names the tool.
        for (name, bytes) in extensions.output_bytes() {
            layered.config.tool_output.entry(name).or_insert(bytes);
        }
        self.extensions = Arc::new(extensions);
        custom::remember(&providers);
        self.custom = providers
            .into_iter()
            .filter(ProviderFile::is_custom)
            .map(|file| (provider_for(&file), file))
            .collect();
        if let Some(note) = layered.layers.untrusted_note() {
            let mut notes = self.notes.lock().unwrap_or_else(|p| p.into_inner());
            if !notes.contains(&note) {
                notes.push(note);
            }
        }
        self.config = layered.config.clone();
        self.layered = layered;
    }

    /// The queued notes (an untrusted project), each handed out once.
    pub fn take_notes(&self) -> Vec<String> {
        let mut notes = self.notes.lock().unwrap_or_else(|p| p.into_inner());
        std::mem::take(&mut *notes)
    }

    /// A command line flag (`model`, `effort`) as the top layer.
    pub fn set_cli(&mut self, key: &str, value: &str) {
        self.layered.set_cli(key, value);
        self.config.model.clone_from(&self.layered.config.model);
        self.config.effort.clone_from(&self.layered.config.effort);
    }

    /// The route of a model of a custom provider; `None` for other models.
    pub(super) fn custom_route(&self, model: &str) -> Option<Result<Route, String>> {
        let (prefix, _) = model.split_once('/')?;
        let (provider, file) = self.custom.iter().find(|(_, file)| file.id == prefix)?;
        Some(self.custom_auth(file).map(|auth| (provider.clone(), auth)))
    }

    /// `keyEnv` (the variable must be set), `account: claude` (the Claude
    /// accounts), or no key at all (a local server).
    fn custom_auth(&self, file: &ProviderFile) -> Result<Arc<dyn AuthSource>, String> {
        if let Some(var) = file.key_env.as_deref() {
            let key = env_value(var).ok_or_else(|| {
                format!("provider {}: set the {var} environment variable", file.id)
            })?;
            return Ok(Arc::new(StaticAuth::new("env-key", var, Auth::ApiKey(key))));
        }
        match file.account.as_deref() {
            Some("claude" | "anthropic") => self.auth(),
            Some(other) => Err(format!("provider {}: unknown account `{other}`", file.id)),
            None => Ok(Arc::new(StaticAuth::new(
                "no-key",
                "no key",
                Auth::ApiKey(String::new()),
            ))),
        }
    }
}

/// `fast-harness doctor`: the folders, every file that loaded with its
/// layer, every file skipped with the reason, the providers, and each key
/// that is not at its default with its source.
pub fn doctor(settings: &Settings) -> String {
    use std::fmt::Write;
    let layered = &settings.layered;
    let mut out = String::new();
    let _ = writeln!(out, "home: {}", layered.layers.home.display());
    let project = match (&layered.layers.project, layered.layers.trusted) {
        (Some(root), true) => format!("{} (trusted)", root.display()),
        (Some(root), false) => format!("{} (not trusted: not loaded)", root.display()),
        (None, _) => "none".into(),
    };
    let _ = writeln!(out, "project: {project}");
    let _ = writeln!(out, "loaded:");
    for file in &layered.report.loaded {
        let _ = writeln!(out, "  {:<8} {}", file.source.word(), file.path.display());
    }
    let _ = writeln!(out, "skipped:");
    for skip in &layered.report.skipped {
        let _ = writeln!(out, "  {}: {}", skip.path.display(), skip.reason);
    }
    let _ = writeln!(out, "providers:");
    for file in &settings.providers {
        let kind = match file.is_custom() {
            true => file.api.clone().unwrap_or_default(),
            false => "overrides of the built-in provider".into(),
        };
        let _ = writeln!(out, "  {} ({kind}, {} models)", file.id, file.models.len());
    }
    extension_lines(&mut out, &settings.index);
    hook_lines(&mut out, &settings.hooks);
    let _ = writeln!(out, "settings not at their default:");
    let changed = layered
        .sources
        .iter()
        .filter(|(_, source)| **source != layers::Source::Default);
    for (key, source) in changed {
        let value = layered
            .get(key)
            .map(|(value, _)| value.to_string())
            .unwrap_or_default();
        let _ = writeln!(out, "  {key} = {value} ({source})");
    }
    out
}

/// The `hooks` section of `doctor`: one line per entry in run order.
fn hook_lines(out: &mut String, hooks: &Hooks) {
    use std::fmt::Write;
    let _ = writeln!(out, "hooks:");
    for (event, entry) in hooks.entries() {
        let matcher = match (event.has_tool(), entry.matcher.as_deref()) {
            (true, Some(pattern)) => format!(" match {pattern}"),
            _ => String::new(),
        };
        let mut command = entry.command.clone();
        for arg in &entry.args {
            command.push(' ');
            command.push_str(arg);
        }
        let source = hooks.source_of(entry).word();
        let timeout = entry.timeout_ms();
        let _ = writeln!(
            out,
            "  {}{matcher}: {command} ({source}, {timeout} ms)",
            event.name()
        );
    }
}

/// The `tools`, `agents` and `commands` sections of `doctor`.
fn extension_lines(out: &mut String, index: &ExtIndex) {
    use std::fmt::Write;
    let sections: [(&str, &[Item]); 4] = [
        ("tools", &index.tools),
        ("agents", &index.agents),
        ("commands", &index.commands),
        ("skills", &index.skills),
    ];
    for (kind, items) in sections {
        let _ = writeln!(out, "{kind}:");
        for item in items {
            let entry = &item.entry;
            let what = match (kind, entry.command, entry.disabled) {
                ("tools", _, true) => "disabled",
                ("tools", true, _) => "command tool",
                ("tools", false, _) => "override",
                _ => "",
            };
            let what = if what.is_empty() {
                String::new()
            } else {
                format!(" [{what}]")
            };
            let (source, path) = (item.source.word(), item.path.display());
            let _ = writeln!(out, "  {}{what} ({source}) {path}", entry.name);
        }
    }
    let _ = writeln!(
        out,
        "index: {} files read, {} from index.json",
        index.reads, index.cached
    );
}
