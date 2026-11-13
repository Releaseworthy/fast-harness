//! Portable core of fast-harness.
//!
//! Layering (a module never imports a higher one):
//! `types` / `theme` / `ui_thread` -> `http` / `sse` / `platform` -> `crypto` / `store` / `net`
//! -> `media` -> `llm` / `tools` / `auth` / `config` -> `trace` / `session` / `events` / `keys`
//! -> `workspaces` / `accounts` -> `agent` -> `view` / `panels` / `stats` -> `reflect`
//! -> `service` -> `ext` -> `app`.
//! `ext` is the native plugin API (PLAN.md D52); the generated
//! `fast-harness-plugins` crate sits above the core and only the binaries
//! depend on it.
//! OS specific code lives only under `platform`.
//! See ARCHITECTURE.md for this layout and docs/decisions/ for the decisions behind it.

pub mod accounts;
pub mod agent;
pub mod app;
pub mod auth;
pub mod config;
pub mod crypto;
pub mod events;
pub mod ext;
pub mod http;
pub mod keys;
pub mod llm;
pub mod media;
pub mod net;
pub mod panels;
pub mod platform;
pub mod reflect;
pub mod service;
pub mod session;
pub mod sse;
pub mod stats;
pub mod store;
pub mod theme;
pub mod tools;
pub mod trace;
pub mod types;
pub mod ui_thread;
pub mod view;
pub mod workspaces;
