//! `Runtime::parts` picks the provider and the credential from the model id
//! (PLAN.md D24): `antigravity/<id>` goes to Antigravity with the Google
//! accounts, any other id to Claude.

use std::path::PathBuf;

use fast_harness_core::accounts::google::SignIn;
use fast_harness_core::accounts::model::{Account, AccountKind, Provider, Secret};
use fast_harness_core::app::{Runtime, NO_GOOGLE_ACCOUNT};

struct Home(PathBuf);

impl Home {
    fn new(tag: &str) -> Home {
        let dir =
            std::env::temp_dir().join(format!("fast-harness-routing-{tag}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        Home(dir)
    }
}

impl Drop for Home {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

fn add_google(rt: &Runtime) {
    let account = Account {
        id: Account::google_id("user-route"),
        kind: AccountKind::Own,
        provider: Provider::Google,
        email: Some("route@example.com".into()),
        name: None,
        plan: None,
        added_at: 1,
    };
    let secret = Secret::Google {
        refresh: "fake-refresh-route".into(),
        access: "fake-access-route".into(),
        expires_at: u64::MAX,
        project_id: Some("proj-route".into()),
    };
    rt.pool.add_google(SignIn { account, secret }).unwrap();
}

#[test]
fn parts_follow_the_model_id() {
    let home = Home::new("both");
    let mut rt = Runtime::load_at(home.0.clone()).unwrap();
    rt.pool.add_api_key("fake-key-route", "route key").unwrap();
    add_google(&rt);
    rt.config.model = "claude-opus-5-5".into();
    let parts = rt.parts(&home.0).unwrap();
    assert_eq!(parts.provider.id(), "anthropic");
    rt.config.model = "antigravity/gemini-3-flash".into();
    let parts = rt.parts(&home.0).unwrap();
    assert_eq!(parts.provider.id(), "antigravity");
    assert_eq!(parts.config.model, "antigravity/gemini-3-flash");
    let (provider, _) = rt.route("antigravity/claude-sonnet-5-5").unwrap();
    assert_eq!(provider.id(), "antigravity");
}

#[test]
fn antigravity_without_a_google_account_names_the_way_to_add_one() {
    let home = Home::new("no-google");
    let mut rt = Runtime::load_at(home.0.clone()).unwrap();
    rt.pool.add_api_key("fake-key-route", "route key").unwrap();
    rt.config.model = "antigravity/gemini-3-flash".into();
    let err = rt.parts(&home.0).err().expect("no Google account");
    assert_eq!(err, NO_GOOGLE_ACCOUNT);
    assert!(err.contains("Accounts > Add Google account"));
    assert!(err.contains("fast-harness accounts login --google"));
    rt.config.model = "claude-opus-5-5".into();
    assert!(rt.parts(&home.0).is_ok());
}
