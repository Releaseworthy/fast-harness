//! Tests of `args.rs`.

use super::*;
fn parse_str(line: &str) -> Result<Command, String> {
    parse(line.split_whitespace().map(String::from).collect())
}

#[test]
fn parses_commands() {
    let Command::Print(print) = parse_str("-p hi --model m --effort low -c --cwd x").unwrap()
    else {
        panic!("not print mode");
    };
    assert_eq!(print.prompt, "hi");
    assert_eq!(print.model.as_deref(), Some("m"));
    assert_eq!(print.effort.as_deref(), Some("low"));
    assert!(print.continue_latest);
    assert_eq!(print.cwd, Some(PathBuf::from("x")));
    assert_eq!(parse_str("--version").unwrap(), Command::Version);
    assert_eq!(parse_str("help").unwrap(), Command::Help);
    assert_eq!(parse_str("").unwrap(), Command::Help);
    assert_eq!(
        parse_str("sessions").unwrap(),
        Command::Sessions { cwd: None }
    );
    let trace = parse_str("trace latest --json").unwrap();
    let id = Some("latest".to_string());
    assert_eq!(
        trace,
        Command::Trace {
            id,
            steps: false,
            json: true,
            cwd: None
        }
    );
    assert_eq!(
        parse_str("trace --steps --cwd d").unwrap(),
        Command::Trace {
            id: None,
            steps: true,
            json: false,
            cwd: Some(PathBuf::from("d"))
        }
    );
    assert!(parse_str("trace a b").is_err());
    assert!(parse_str("-p").is_err());
    assert!(parse_str("--bogus").is_err());
    assert!(parse_str("frob").is_err());
    assert!(parse_str("-p x -c --resume y").is_err());
    let tui = parse_str("tui -c --cwd d").unwrap();
    let expected = TuiArgs {
        continue_latest: true,
        resume: None,
        cwd: Some(PathBuf::from("d")),
    };
    assert_eq!(tui, Command::Tui(expected));
    assert!(parse_str("tui -c --resume x").is_err());
    assert!(parse_str("tui -p hi").is_err());
    assert!(parse_str("tui extra").is_err());
}

#[test]
fn parses_accounts() {
    let accounts = |line: &str| match parse_str(line) {
        Ok(Command::Accounts(action)) => Ok(action),
        Ok(other) => panic!("not accounts: {other:?}"),
        Err(err) => Err(err),
    };
    assert_eq!(accounts("accounts").unwrap(), AccountsAction::List);
    assert_eq!(accounts("accounts list").unwrap(), AccountsAction::List);
    let login = accounts("accounts login --no-browser").unwrap();
    assert_eq!(login, AccountsAction::Login { browser: false });
    assert_eq!(
        accounts("accounts use u-1").unwrap(),
        AccountsAction::Use("u-1".into())
    );
    let remove = accounts("accounts remove a@b.c").unwrap();
    assert_eq!(remove, AccountsAction::Remove("a@b.c".into()));
    let key = accounts("accounts add-key work").unwrap();
    assert_eq!(
        key,
        AccountsAction::AddKey {
            label: Some("work".into())
        }
    );
    assert_eq!(accounts("accounts usage").unwrap(), AccountsAction::Usage);
    // Google sign-in and its OAuth client moved to branch `antigravity` (D53).
    assert!(accounts("accounts login --google").is_err());
    assert!(accounts("accounts google-client --id test-client.apps.example").is_err());
    assert!(accounts("accounts list --id x").is_err());
    assert!(accounts("accounts use").is_err());
    assert!(accounts("accounts frob").is_err());
    assert!(accounts("accounts use a b").is_err());
}

#[test]
fn parses_models() {
    assert_eq!(
        parse_str("models").unwrap(),
        Command::Models { json: false }
    );
    let json = parse_str("models --json").unwrap();
    assert_eq!(json, Command::Models { json: true });
    assert!(parse_str("models extra").is_err());
}

#[test]
fn parses_prompt() {
    let cwd = Some(PathBuf::from("d"));
    assert_eq!(parse_str("prompt").unwrap(), Command::Prompt { cwd: None });
    assert_eq!(
        parse_str("prompt --cwd d").unwrap(),
        Command::Prompt { cwd }
    );
    assert!(parse_str("prompt extra").is_err());
    assert!(USAGE.contains("fast-harness prompt [--cwd DIR]"));
}

/// `trust --cwd DIR` trusts DIR, not the current directory (D63).
#[test]
fn trust_takes_cwd_as_its_path() {
    let path = Some(PathBuf::from("d"));
    assert_eq!(
        parse_str("trust --cwd d").unwrap(),
        Command::Trust { path: path.clone() }
    );
    assert_eq!(parse_str("trust d").unwrap(), Command::Trust { path });
    assert_eq!(parse_str("trust").unwrap(), Command::Trust { path: None });
    assert!(parse_str("trust d --cwd e").is_err(), "one path only");
    assert!(USAGE.contains("trust [PATH|--cwd DIR]") && USAGE.contains("AGENTS.md files load"));
}

#[test]
fn parses_serve_and_json_print() {
    let Command::Print(print) = parse_str("-p hi --json").unwrap() else {
        panic!("not print mode");
    };
    assert!(print.json);
    assert_eq!(
        parse_str("serve --stdio --model m").unwrap(),
        Command::Serve {
            model: Some("m".into()),
            effort: None
        }
    );
    assert!(parse_str("serve").is_err());
    assert!(USAGE.contains("fast-harness serve --stdio"));
}

#[test]
fn parses_session_commands() {
    let Command::Session { action, cwd } =
        parse_str("send latest look here --queue --wait --cwd d").unwrap()
    else {
        panic!("not a session command");
    };
    assert_eq!(cwd, Some(PathBuf::from("d")));
    let want = SessionAction::Send {
        id: "latest".into(),
        text: "look here".into(),
        queue: true,
        wait: true,
    };
    assert_eq!(action, want);
    let start = parse_str("start --model m1 fix the bug").unwrap();
    let want = SessionAction::Start {
        prompt: "fix the bug".into(),
        model: Some("m1".into()),
    };
    assert_eq!(
        start,
        Command::Session {
            action: want,
            cwd: None
        }
    );
    assert!(matches!(
        parse_str("ps --json").unwrap(),
        Command::Session {
            action: SessionAction::Ps { json: true },
            ..
        }
    ));
    assert!(parse_str("send x hi --steer --queue").is_err());
    assert!(parse_str("send x").is_err());
    assert!(parse_str("stop a b").is_err());
    assert!(parse_str("tail x --json").is_ok());
}
