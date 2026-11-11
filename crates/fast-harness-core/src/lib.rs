//! Portable core of fast-harness.
//!
//! Layering (a module never imports a higher one):
//! `types` -> `http` / `sse` / `platform` -> `llm` / `tools` / `auth` -> `trace` / `session`
//! -> `accounts` -> `agent` -> `view` / `app`. OS specific code lives only under `platform`.
//! See PLAN.md for the decisions behind this layout.

pub mod accounts;
pub mod agent;
pub mod app;
pub mod auth;
pub mod config;
pub mod events;
pub mod http;
pub mod llm;
pub mod net;
pub mod platform;
pub mod session;
pub mod sse;
pub mod theme;
pub mod tools;
pub mod trace;
pub mod types;
pub mod view;
