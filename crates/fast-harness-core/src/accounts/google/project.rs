//! The account's Cloud Code project and plan: `loadCodeAssist`, and
//! `onboardUser` with its operation polling when no project is named, as the
//! antigravity-switch plugin does it.

use super::{text, GoogleApi, GoogleError};
use crate::http::Transport;
use serde_json::{json, Value};
use std::thread;

/// The project used when Google names none after onboarding (business accounts).
pub const FALLBACK_PROJECT_ID: &str = "rising-fact-p41fc";
/// `onboardUser` extras of Antigravity's control plane.
const NODE_API_CLIENT: &str = "google-api-nodejs-client/10.3.0";
const GOOG_API_CLIENT: &str = "gl-node/22.21.1";
/// `onboardUser` polls at most this often.
const ONBOARD_ATTEMPTS: usize = 10;
/// Display names of the known tier ids.
const TIER_NAMES: [(&str, &str); 4] = [
    ("free-tier", "Free"),
    ("standard-tier", "Standard"),
    ("g1-pro-tier", "Google AI Pro"),
    ("g1-ultra-tier", "Google AI Ultra"),
];

/// What `loadCodeAssist` (and `onboardUser`) said.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct ProjectInfo {
    /// `None` only when `loadCodeAssist` failed (and no hint was given).
    pub project_id: Option<String>,
    /// Tier id: `paidTier`, else `currentTier`, else the default allowed tier.
    pub tier: Option<String>,
    /// Its display name, the account's plan.
    pub tier_name: Option<String>,
}

/// A project as a string or an object with `id`.
fn project_of(value: &Value) -> Option<String> {
    text(value).or_else(|| text(&value["id"]))
}

/// The tier object with an id: paid, else current, else the default allowed one.
fn tier_info(reply: &Value) -> Option<&Value> {
    let with_id = |tier: &'_ Value| text(&tier["id"]).is_some();
    let allowed = reply["allowedTiers"].as_array();
    [&reply["paidTier"], &reply["currentTier"]]
        .into_iter()
        .find(|tier| with_id(tier))
        .or_else(|| {
            allowed?
                .iter()
                .find(|t| t["isDefault"] == Value::Bool(true))
        })
}

/// The tier `onboardUser` asks for.
fn onboard_tier(reply: &Value) -> String {
    let allowed = reply["allowedTiers"].as_array();
    let default = allowed.and_then(|a| a.iter().find(|t| t["isDefault"] == Value::Bool(true)));
    default
        .and_then(|t| text(&t["id"]))
        .or_else(|| allowed.and_then(|a| a.first()).and_then(|t| text(&t["id"])))
        .or_else(|| text(&reply["currentTier"]["id"]))
        .unwrap_or_else(|| "free-tier".into())
}

/// The display name of a tier.
fn tier_name(info: &Value, id: Option<&str>) -> Option<String> {
    text(&info["name"]).or_else(|| {
        let id = id?;
        let known = TIER_NAMES.iter().find(|(key, _)| *key == id);
        Some(known.map_or(id, |(_, name)| *name).to_string())
    })
}

impl GoogleApi {
    /// The project and tier of `access`. A failed `loadCodeAssist` gives
    /// `hint` and no tier; a reply without a project runs `onboardUser` when
    /// `allow_onboard`, then falls back to [`FALLBACK_PROJECT_ID`].
    pub fn load_project(
        &self,
        http: &dyn Transport,
        access: &str,
        hint: Option<&str>,
        allow_onboard: bool,
    ) -> Result<ProjectInfo, GoogleError> {
        let body = json!({"metadata": {"ideType": "ANTIGRAVITY"}});
        let answer = self
            .hub
            .control_call(http, access, "loadCodeAssist", &body, &[])?;
        let reply = answer.json().filter(|_| answer.status == 200);
        let Some(reply) = reply.filter(Value::is_object) else {
            return Ok(ProjectInfo {
                project_id: hint.map(str::to_string),
                ..ProjectInfo::default()
            });
        };
        let info = tier_info(&reply);
        let tier = info.and_then(|t| text(&t["id"]));
        let tier_name = info.and_then(|t| tier_name(t, tier.as_deref()));
        let mut project_id = project_of(&reply["cloudaicompanionProject"]);
        if project_id.is_none() && allow_onboard {
            project_id = self.onboard(http, access, &onboard_tier(&reply))?;
        }
        let project_id = project_id
            .or_else(|| hint.map(str::to_string))
            .unwrap_or_else(|| FALLBACK_PROJECT_ID.to_string());
        Ok(ProjectInfo {
            project_id: Some(project_id),
            tier,
            tier_name,
        })
    }

    /// `onboardUser` until its operation is done (at most 10 polls).
    fn onboard(
        &self,
        http: &dyn Transport,
        access: &str,
        tier: &str,
    ) -> Result<Option<String>, GoogleError> {
        let version = self.hub.version(http);
        let agent = format!(
            "{} {NODE_API_CLIENT}",
            crate::llm::antigravity::hub::hub_user_agent(&version)
        );
        let extra = [
            ("user-agent", agent.as_str()),
            ("x-goog-api-client", GOOG_API_CLIENT),
            ("accept", "*/*"),
        ];
        let body = json!({
            "tier_id": tier,
            "metadata": {"ide_type": "ANTIGRAVITY", "ide_version": version, "ide_name": "antigravity"},
        });
        for attempt in 0..ONBOARD_ATTEMPTS {
            if attempt > 0 {
                thread::sleep(self.onboard_wait);
            }
            let answer = self
                .hub
                .control_call(http, access, "onboardUser", &body, &extra)?;
            if answer.status != 200 {
                return Ok(None);
            }
            let reply = answer.json().unwrap_or(Value::Null);
            if reply["done"] == Value::Bool(true) {
                return Ok(project_of(&reply["response"]["cloudaicompanionProject"]));
            }
        }
        Ok(None)
    }
}
