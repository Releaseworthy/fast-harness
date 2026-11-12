//! `fast-harness accounts`: list, sign in, choose, remove, add an API key, read usage.

use std::io::BufRead;
use std::process::ExitCode;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Duration;

use fast_harness_core::accounts::model::{now_unix_ms, AccountKind};
use fast_harness_core::accounts::oauth;
use fast_harness_core::accounts::pool::{AccountPool, AccountStatus};
use fast_harness_core::accounts::usage::Window;
use fast_harness_core::net::UreqTransport;
use fast_harness_core::platform;

use crate::args::AccountsAction;
use crate::auth_choice::runtime;

static CANCEL: AtomicBool = AtomicBool::new(false);

fn on_ctrl_c() {
    if CANCEL.swap(true, Ordering::SeqCst) {
        std::process::exit(130);
    }
}

/// How long `fast-harness accounts login` waits for the browser.
const LOGIN_TIMEOUT: Duration = Duration::from_secs(600);

pub fn run(action: AccountsAction) -> Result<ExitCode, String> {
    let rt = runtime()?;
    let pool = rt.pool.as_ref();
    match action {
        AccountsAction::List => print_list(pool),
        AccountsAction::Login { browser } => login(pool, browser)?,
        AccountsAction::Use(name) => {
            let id = resolve(pool, &name)?;
            pool.set_active(&id).map_err(|err| err.to_string())?;
            print_list(pool);
        }
        AccountsAction::Remove(name) => {
            let id = resolve(pool, &name)?;
            pool.remove(&id).map_err(|err| err.to_string())?;
            println!("removed {id}");
        }
        AccountsAction::AddKey { label } => add_key(pool, label.as_deref())?,
        AccountsAction::Usage => {
            let read = pool.poll_usage(rt.transport.as_ref());
            println!("{read} usage read(s)");
            print_list(pool);
        }
    }
    Ok(ExitCode::SUCCESS)
}

fn login(pool: &AccountPool, browser: bool) -> Result<(), String> {
    let flow = oauth::begin_login().map_err(|err| err.to_string())?;
    eprintln!(
        "Sign in to Claude in the browser. If it does not open, visit:\n{}",
        flow.url()
    );
    if browser {
        if let Err(err) = platform::open_url(flow.url()) {
            eprintln!("[note] could not open the browser: {err}");
        }
    }
    let _ = platform::on_interrupt(on_ctrl_c);
    let http = UreqTransport::new();
    let account = pool
        .finish_login(&http, &flow, LOGIN_TIMEOUT, &CANCEL)
        .map_err(|err| err.to_string())?;
    println!(
        "signed in: {} ({})",
        label(&account.email, &account.name),
        account.id
    );
    print_list(pool);
    Ok(())
}

fn add_key(pool: &AccountPool, label: Option<&str>) -> Result<(), String> {
    eprintln!("Paste the Anthropic API key and press Enter:");
    let mut line = String::new();
    std::io::stdin()
        .lock()
        .read_line(&mut line)
        .map_err(|err| format!("stdin: {err}"))?;
    let account = pool
        .add_api_key(line.trim(), label.unwrap_or("API key"))
        .map_err(|err| err.to_string())?;
    println!("added {}", account.id);
    Ok(())
}

/// An account id, an e-mail address (any case) or a unique id prefix.
fn resolve(pool: &AccountPool, name: &str) -> Result<String, String> {
    let list = pool.status();
    let lower = name.to_lowercase();
    let exact = list.iter().find(|s| {
        s.account.id == name
            || s.account.email.as_deref().map(str::to_lowercase) == Some(lower.clone())
    });
    if let Some(status) = exact {
        return Ok(status.account.id.clone());
    }
    let prefixed: Vec<_> = list
        .iter()
        .filter(|s| s.account.id.starts_with(name))
        .collect();
    match prefixed.as_slice() {
        [one] => Ok(one.account.id.clone()),
        [] => Err(format!(
            "no account matches `{name}` (see `fast-harness accounts`)"
        )),
        _ => Err(format!("`{name}` matches {} accounts", prefixed.len())),
    }
}

pub(crate) fn label(email: &Option<String>, name: &Option<String>) -> String {
    email
        .clone()
        .or_else(|| name.clone())
        .unwrap_or_else(|| "-".into())
}

fn print_list(pool: &AccountPool) {
    let claude = pool.status();
    if claude.is_empty() {
        println!("no accounts: `fast-harness accounts login` or `fast-harness accounts add-key`");
        return;
    }
    let now = i64::try_from(now_unix_ms()).unwrap_or(i64::MAX);
    println!(
        "  {:<14} {:<7} {:<30} {:<6} {:>5} {:>5}  {:<10} STATE",
        "ID", "KIND", "ACCOUNT", "PLAN", "5H", "7D", "7D RESET"
    );
    for status in &claude {
        println!("{}", row(status, now));
    }
    if let Some(warning) = pool.save_error() {
        println!("warning: accounts: {warning}");
    }
}
fn row(status: &AccountStatus, now: i64) -> String {
    let account = &status.account;
    let kind = match account.kind {
        AccountKind::Own => "own",
        AccountKind::ApiKey => "key",
    };
    let usage = status.usage.as_ref();
    let five = usage.and_then(|u| u.five_hour.as_ref());
    let seven = usage.and_then(|u| u.seven_day.as_ref());
    let reset = seven.and_then(|w| w.resets_at).map(|at| relative(at - now));
    format!(
        "{} {:<14} {:<7} {:<30} {:<6} {:>5} {:>5}  {:<10} {}",
        if status.active { "*" } else { " " },
        clip(&account.id, 14),
        kind,
        clip(&label(&account.email, &account.name), 30),
        clip(account.plan.as_deref().unwrap_or("-"), 6),
        percent(five),
        percent(seven),
        reset.unwrap_or_else(|| "-".into()),
        state(status, now)
    )
}

pub(crate) fn percent(window: Option<&Window>) -> String {
    window.map_or_else(|| "-".into(), |w| format!("{:.0}%", w.utilization))
}

pub(crate) fn state(status: &AccountStatus, now: i64) -> String {
    if let Some(until) = status.held_until {
        let reason = status.hold_reason.as_deref().unwrap_or("held");
        return format!("held {} ({reason})", relative(until - now));
    }
    if status.token_ok {
        "ok".into()
    } else {
        "no usable token".into()
    }
}

/// `in 2d 3h`, `in 45m`, `now`.
pub(crate) fn relative(ms: i64) -> String {
    if ms <= 0 {
        return "now".into();
    }
    let minutes = ms / 60_000;
    let (days, hours, mins) = (minutes / 1440, minutes / 60 % 24, minutes % 60);
    match (days, hours) {
        (0, 0) => format!("in {mins}m"),
        (0, _) => format!("in {hours}h {mins}m"),
        _ => format!("in {days}d {hours}h"),
    }
}

fn clip(text: &str, max: usize) -> String {
    if text.chars().count() <= max {
        return text.to_string();
    }
    let kept: String = text.chars().take(max.saturating_sub(1)).collect();
    format!("{kept}~")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn relative_times() {
        assert_eq!(relative(-5), "now");
        assert_eq!(relative(45 * 60_000), "in 45m");
        assert_eq!(relative((3 * 60 + 5) * 60_000), "in 3h 5m");
        assert_eq!(relative((2 * 1440 + 3 * 60) * 60_000), "in 2d 3h");
        assert_eq!(clip("abcdef", 4), "abc~");
        assert_eq!(clip("abc", 4), "abc");
    }
}
