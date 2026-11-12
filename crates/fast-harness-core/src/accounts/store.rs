//! `~/.fast-harness/accounts.json` with encrypted secrets.
//!
//! Schema (version 3):
//! `{"version":3,"active":<id or null>,
//! "accounts":[{...account fields,"provider":"anthropic","secret":"<base64>"}]}`.
//! `active` points at an Anthropic account.
//! A secret is base64(platform::protect(serde_json bytes of [`Secret`])). Unknown
//! top-level keys (such as `activeGoogle`) are kept on save, and so are records
//! of a provider this build does not know (`"google"`, whose code moved to
//! branch `antigravity`, PLAN.md D53): they stay raw JSON, are never listed and
//! are written back unchanged after the known accounts.
//!
//! Version 1 is version 2 plus accounts of kind `linked` (claude-switch links,
//! removed 2026-11-11, PLAN.md D8). They never held a secret: loading a version
//! 1 file drops them (and an `active` pointing at one falls back to the first
//! remaining account); the next save writes version 3.
//!
//! Version 2 is version 3 without `provider`: its records
//! load as `anthropic` (PLAN.md D26).

use crate::accounts::crypto::{base64_decode, base64_encode, Base64Error};
use crate::accounts::model::{Account, AccountKind, Provider, Secret};
use crate::platform::{self, PlatformError};
use serde::{Deserialize, Serialize};
use serde_json::{Map, Value};
use std::fmt;
use std::fs;
use std::io;
use std::path::{Path, PathBuf};

/// The schema version this code reads and writes.
pub const VERSION: u64 = 3;
/// Older versions this code still reads (see the module docs).
pub const VERSION_V2: u64 = 2;
pub const VERSION_V1: u64 = 1;

/// Errors of the account store. None of them carries a secret value.
#[derive(Debug)]
pub enum StoreError {
    Io(io::Error),
    /// `accounts.json` is not valid JSON or does not match the schema.
    Json(serde_json::Error),
    UnsupportedVersion(u64),
    Platform(PlatformError),
    /// The stored secret of this account cannot be decoded or decrypted.
    BadSecret(String),
    UnknownAccount(String),
    /// The secret type does not fit the account kind.
    SecretKindMismatch(String),
}

impl fmt::Display for StoreError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            StoreError::Io(err) => write!(f, "accounts file: {err}"),
            StoreError::Json(err) => write!(f, "accounts file is invalid: {err}"),
            StoreError::UnsupportedVersion(v) => {
                write!(f, "accounts file version {v} is not supported")
            }
            StoreError::Platform(err) => write!(f, "secret encryption: {err}"),
            StoreError::BadSecret(id) => write!(f, "the secret of account {id} cannot be read"),
            StoreError::UnknownAccount(id) => write!(f, "no account {id}"),
            StoreError::SecretKindMismatch(id) => {
                write!(f, "secret type does not fit account {id}")
            }
        }
    }
}

impl std::error::Error for StoreError {}

impl From<io::Error> for StoreError {
    fn from(err: io::Error) -> Self {
        StoreError::Io(err)
    }
}

impl From<PlatformError> for StoreError {
    fn from(err: PlatformError) -> Self {
        StoreError::Platform(err)
    }
}

#[derive(Clone, Serialize, Deserialize)]
struct Entry {
    #[serde(flatten)]
    account: Account,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    secret: Option<String>,
}

#[derive(Serialize)]
struct FileData {
    version: u64,
    #[serde(default)]
    active: Option<String>,
    #[serde(default)]
    accounts: Vec<Value>,
    #[serde(flatten)]
    extra: Map<String, Value>,
}

/// The file as read: accounts stay raw JSON until the version is known.
#[derive(Deserialize)]
struct RawFile {
    version: u64,
    #[serde(default)]
    active: Option<String>,
    #[serde(default)]
    accounts: Vec<Value>,
    #[serde(flatten)]
    extra: Map<String, Value>,
}

/// Whether `record` names a provider other than `anthropic` (a missing
/// `provider` is `anthropic`, as in version 2).
fn is_foreign(record: &Value) -> bool {
    record
        .get("provider")
        .and_then(Value::as_str)
        .is_some_and(|p| p != "anthropic")
}

/// The entries of a file of `version` and its records of unknown providers;
/// version 1 `linked` records are dropped.
fn entries_of(version: u64, accounts: Vec<Value>) -> Result<(Vec<Entry>, Vec<Value>), StoreError> {
    if ![VERSION, VERSION_V2, VERSION_V1].contains(&version) {
        return Err(StoreError::UnsupportedVersion(version));
    }
    let (foreign, known): (Vec<Value>, Vec<Value>) = accounts
        .into_iter()
        .filter(|a| {
            version != VERSION_V1 || a.get("kind").and_then(Value::as_str) != Some("linked")
        })
        .partition(is_foreign);
    let entries = known
        .into_iter()
        .map(|a| serde_json::from_value(a).map_err(StoreError::Json))
        .collect::<Result<Vec<Entry>, StoreError>>()?;
    Ok((entries, foreign))
}

/// The accounts file in memory. Changes reach the disk on [`AccountStore::save`].
pub struct AccountStore {
    path: PathBuf,
    active: Option<String>,
    entries: Vec<Entry>,
    /// Records of providers this build does not know, kept as read.
    foreign: Vec<Value>,
    extra: Map<String, Value>,
}

impl fmt::Debug for AccountStore {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let ids: Vec<&str> = self.entries.iter().map(|e| e.account.id.as_str()).collect();
        f.debug_struct("AccountStore")
            .field("path", &self.path)
            .field("active", &self.active)
            .field("accounts", &ids)
            .field("foreign", &self.foreign.len())
            .finish()
    }
}

/// `platform::app_home()/accounts.json`, or `None` without a home directory.
pub fn default_path() -> Option<PathBuf> {
    platform::app_home().map(|home| home.join("accounts.json"))
}

impl AccountStore {
    /// Reads `path`. A missing file is an empty store.
    pub fn load(path: impl Into<PathBuf>) -> Result<AccountStore, StoreError> {
        let path = path.into();
        let text = match fs::read_to_string(&path) {
            Ok(text) => text,
            Err(err) if err.kind() == io::ErrorKind::NotFound => String::new(),
            Err(err) => return Err(err.into()),
        };
        let mut store = AccountStore {
            path,
            active: None,
            entries: Vec::new(),
            foreign: Vec::new(),
            extra: Map::new(),
        };
        if text.trim().is_empty() {
            return Ok(store);
        }
        let data: RawFile = serde_json::from_str(&text).map_err(StoreError::Json)?;
        (store.entries, store.foreign) = entries_of(data.version, data.accounts)?;
        let pointed = data.active.is_some();
        store.active = data.active.filter(|id| {
            data.version != VERSION_V1 || store.entries.iter().any(|e| &e.account.id == id)
        });
        if pointed && store.active.is_none() {
            store.active = store.entries.first().map(|e| e.account.id.clone());
        }
        store.extra = data.extra;
        Ok(store)
    }

    /// The file this store reads and writes.
    pub fn path(&self) -> &Path {
        &self.path
    }

    /// Writes the store: a temporary file in the same folder, renamed over the old one.
    pub fn save(&self) -> Result<(), StoreError> {
        let mut accounts = self
            .entries
            .iter()
            .map(serde_json::to_value)
            .collect::<Result<Vec<Value>, _>>()
            .map_err(StoreError::Json)?;
        accounts.extend(self.foreign.iter().cloned());
        let data = FileData {
            version: VERSION,
            active: self.active.clone(),
            accounts,
            extra: self.extra.clone(),
        };
        let mut text = serde_json::to_string_pretty(&data).map_err(StoreError::Json)?;
        text.push('\n');
        if let Some(dir) = self.path.parent().filter(|d| !d.as_os_str().is_empty()) {
            fs::create_dir_all(dir)?;
        }
        let tmp = self.temp_path();
        if let Err(err) = fs::write(&tmp, text).and_then(|()| fs::rename(&tmp, &self.path)) {
            let _ = fs::remove_file(&tmp);
            return Err(err.into());
        }
        Ok(())
    }

    fn temp_path(&self) -> PathBuf {
        let mut name = self
            .path
            .file_name()
            .map(|n| n.to_os_string())
            .unwrap_or_else(|| "accounts.json".into());
        name.push(format!(".{}.tmp", std::process::id()));
        self.path.with_file_name(name)
    }

    /// Every account, in file order.
    pub fn list(&self) -> Vec<&Account> {
        self.entries.iter().map(|e| &e.account).collect()
    }

    pub fn get(&self, id: &str) -> Option<&Account> {
        self.entry(id).map(|e| &e.account)
    }

    fn entry(&self, id: &str) -> Option<&Entry> {
        self.entries.iter().find(|e| e.account.id == id)
    }

    fn entry_mut(&mut self, id: &str) -> Option<&mut Entry> {
        self.entries.iter_mut().find(|e| e.account.id == id)
    }

    /// Adds `account` or replaces the one with its id. With `secret: None` an
    /// existing secret is kept.
    pub fn upsert(&mut self, account: Account, secret: Option<Secret>) -> Result<(), StoreError> {
        let sealed = match &secret {
            Some(secret) => Some(seal(&account, secret)?),
            None => None,
        };
        match self.entry_mut(&account.id) {
            Some(entry) => {
                entry.account = account;
                if sealed.is_some() {
                    entry.secret = sealed;
                }
            }
            None => self.entries.push(Entry {
                account,
                secret: sealed,
            }),
        }
        Ok(())
    }

    /// Removes an account (and clears it as active). Returns it if it existed.
    pub fn remove(&mut self, id: &str) -> Option<Account> {
        let index = self.entries.iter().position(|e| e.account.id == id)?;
        if self.active.as_deref() == Some(id) {
            self.active = None;
        }
        Some(self.entries.remove(index).account)
    }

    /// The active Anthropic account, if it is set and still exists.
    pub fn active(&self) -> Option<&Account> {
        self.active_for(Provider::Anthropic)
    }

    /// The active account of `provider`, if it is set and still exists.
    pub fn active_for(&self, provider: Provider) -> Option<&Account> {
        let pointer = match provider {
            Provider::Anthropic => &self.active,
        };
        pointer
            .as_deref()
            .and_then(|id| self.get(id))
            .filter(|a| a.provider == provider)
    }

    /// Makes `id` the active account of its own provider.
    pub fn set_active(&mut self, id: &str) -> Result<(), StoreError> {
        let Some(account) = self.get(id) else {
            return Err(StoreError::UnknownAccount(id.to_string()));
        };
        let pointer = match account.provider {
            Provider::Anthropic => &mut self.active,
        };
        *pointer = Some(id.to_string());
        Ok(())
    }

    /// The decrypted secret of `id`, `None` if it has none.
    pub fn secret(&self, id: &str) -> Result<Option<Secret>, StoreError> {
        let entry = self
            .entry(id)
            .ok_or_else(|| StoreError::UnknownAccount(id.to_string()))?;
        match &entry.secret {
            Some(sealed) => open(id, sealed).map(Some),
            None => Ok(None),
        }
    }

    /// Replaces the secret of `id`.
    pub fn set_secret(&mut self, id: &str, secret: Secret) -> Result<(), StoreError> {
        let entry = self
            .entry_mut(id)
            .ok_or_else(|| StoreError::UnknownAccount(id.to_string()))?;
        entry.secret = Some(seal(&entry.account, &secret)?);
        Ok(())
    }
}

/// Checks that `secret` fits `account`, then encrypts it for the file.
fn seal(account: &Account, secret: &Secret) -> Result<String, StoreError> {
    let fits = matches!(
        (account.provider, account.kind, secret),
        (Provider::Anthropic, AccountKind::Own, Secret::Own { .. })
            | (
                Provider::Anthropic,
                AccountKind::ApiKey,
                Secret::ApiKey { .. }
            )
    );
    if !fits {
        return Err(StoreError::SecretKindMismatch(account.id.clone()));
    }
    let mut plain = serde_json::to_vec(secret).map_err(StoreError::Json)?;
    let blob = platform::protect(&plain);
    plain.fill(0);
    Ok(base64_encode(&blob?))
}

/// Decrypts a stored secret. Errors never include the decrypted bytes.
fn open(id: &str, sealed: &str) -> Result<Secret, StoreError> {
    let blob =
        base64_decode(sealed).map_err(|_: Base64Error| StoreError::BadSecret(id.to_string()))?;
    let mut plain = platform::unprotect(&blob)?;
    let secret = serde_json::from_slice(&plain).map_err(|_| StoreError::BadSecret(id.to_string()));
    plain.fill(0);
    secret
}
