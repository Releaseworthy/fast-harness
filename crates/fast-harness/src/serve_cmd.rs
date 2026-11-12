//! `fast-harness serve --stdio` (PLAN.md D48): JSON-RPC 2.0 on stdin and
//! stdout with the Agent Client Protocol v2 names (docs/service.md). Only
//! protocol messages go to stdout; notes go to stderr.

use std::process::ExitCode;
use std::sync::Arc;

use fast_harness_core::app::RuntimeHost;
use fast_harness_core::service::{rpc, Service};

use crate::auth_choice::runtime;

pub fn run(model: Option<String>, effort: Option<String>) -> Result<ExitCode, String> {
    let mut rt = runtime()?;
    // Served sessions are long-lived: `cacheTtl: auto` caches 1 hour (D36).
    rt.interactive = true;
    let cli: Vec<(String, String)> = [("model", model), ("effort", effort)]
        .into_iter()
        .filter_map(|(key, value)| value.map(|value| (key.to_string(), value)))
        .collect();
    let service = Arc::new(Service::new(Arc::new(RuntimeHost::new(rt, cli))));
    eprintln!("[note] fast-harness serve: JSON-RPC 2.0 on stdin/stdout (ACP v2)");
    let stdin = std::io::stdin().lock();
    rpc::serve(service, stdin, rpc::out(std::io::stdout()));
    Ok(ExitCode::SUCCESS)
}
