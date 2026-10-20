//! The accounts, in a TOML file of `[[account]]` tables, and their
//! passwords: kept by letterbox, in the system's store or its vault, or
//! printed by a command of your own.

use crate::secret::{self, Keeper};
use anyhow::{Context, bail};
use serde::Deserialize;
use std::path::{Path, PathBuf};
use std::process::Command;

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Security {
    /// TLS from the start, on port 993.
    #[default]
    Tls,
    /// Plain, then TLS once the server is asked for it, on port 143.
    Starttls,
    /// None at all: for a server on this machine only, as a bridge or a test.
    None,
}

/// How an account signs in.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Auth {
    /// With a password, given to the server.
    #[default]
    Password,
    /// On Microsoft's page, as Microsoft 365 and Outlook.com have it; a
    /// token renewed, not a password, is kept.
    Microsoft,
}

#[derive(Clone, Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Account {
    pub name: String,
    pub email: String,
    /// The IMAP server.
    pub host: String,
    pub port: Option<u16>,
    /// The name signed in with, if not the address.
    pub user: Option<String>,
    #[serde(default)]
    pub security: Security,
    #[serde(default)]
    pub auth: Auth,
    /// Microsoft's tenant and client ID signed in with, if not any
    /// organisation's and Thunderbird's.
    pub tenant: Option<String>,
    pub client_id: Option<String>,
    /// Where letterbox keeps its password: the system's store, unless
    /// `"vault"`.
    #[serde(default)]
    pub keep: Keeper,
    /// A shell command that prints the password, as `pass show mail/work`,
    /// instead.
    pub password_command: Option<String>,
    /// The password as typed, or Microsoft's refresh token as given, while
    /// an account is being added: never written to the file.
    #[serde(skip)]
    pub password: Option<String>,
    /// Why the password could not be found before the TUI started, not to
    /// be asked for again while it has the terminal.
    #[serde(skip)]
    pub missing: Option<String>,
}

#[derive(Debug, Default, Deserialize)]
#[serde(deny_unknown_fields)]
struct File {
    #[serde(default)]
    account: Vec<Account>,
}

impl Account {
    pub fn tenant(&self) -> &str {
        self.tenant.as_deref().unwrap_or(crate::oauth::TENANT)
    }

    pub fn client(&self) -> &str {
        self.client_id.as_deref().unwrap_or(crate::oauth::CLIENT_ID)
    }

    pub fn user(&self) -> &str {
        self.user.as_deref().unwrap_or(&self.email)
    }

    pub fn port(&self) -> u16 {
        self.port.unwrap_or(match self.security {
            Security::Tls => 993,
            Security::Starttls | Security::None => 143,
        })
    }

    /// The password: printed by `password_command`, or kept by letterbox.
    pub fn password(&self) -> anyhow::Result<String> {
        if let Some(p) = &self.password {
            return Ok(p.clone());
        }
        if let Some(why) = &self.missing {
            bail!("{why}");
        }
        let Some(command) = &self.password_command else {
            return secret::find(self.keep, &self.host, self.user());
        };
        let out = if cfg!(windows) {
            Command::new("cmd").args(["/C", command]).output()
        } else {
            Command::new("sh").args(["-c", command]).output()
        }
        .with_context(|| format!("cannot run {command}"))?;
        if !out.status.success() {
            bail!(
                "{command} gave no password: {}",
                String::from_utf8_lossy(&out.stderr).trim()
            );
        }
        Ok(first_line(&out.stdout))
    }
}

fn first_line(out: &[u8]) -> String {
    String::from_utf8_lossy(out)
        .lines()
        .next()
        .unwrap_or_default()
        .to_string()
}

/// `$XDG_CONFIG_HOME/letterbox/accounts.toml`, else
/// `~/.config/letterbox/accounts.toml`, on macOS too.
pub fn default_path() -> PathBuf {
    let base = std::env::var_os("XDG_CONFIG_HOME")
        .filter(|d| !d.is_empty())
        .map(PathBuf::from)
        .unwrap_or_else(|| {
            PathBuf::from(std::env::var_os("HOME").unwrap_or_default()).join(".config")
        });
    base.join("letterbox/accounts.toml")
}

/// The accounts in the file at `path`; none if there is no file.
pub fn load(path: &Path) -> anyhow::Result<Vec<Account>> {
    let text = match std::fs::read_to_string(path) {
        Ok(t) => t,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(Vec::new()),
        Err(e) => return Err(e).with_context(|| format!("cannot read {}", path.display())),
    };
    let file: File =
        toml::from_str(&text).with_context(|| format!("cannot read {}", path.display()))?;
    for a in &file.account {
        if a.security == Security::None && !local(&a.host) {
            bail!(
                "{}: security = \"none\" sends the password unencrypted; it is for a server \
                 on this machine only, not {}",
                a.name,
                a.host
            );
        }
    }
    Ok(file.account)
}

fn local(host: &str) -> bool {
    matches!(host, "localhost" | "127.0.0.1" | "::1")
}

/// What to know of signing in with Microsoft.
pub const MICROSOFT_NOTE: &str = "Signs in on Microsoft's page, in your browser, or with a code \
     on any device: no password is kept here, only a token, renewed.";

/// The IMAP server of an address, as the large providers have it, else
/// `imap.` and its domain; and what to know to sign in there.
pub fn guess(email: &str) -> (String, Option<&'static str>) {
    let domain = email
        .rsplit_once('@')
        .map(|(_, d)| d.trim().to_lowercase())
        .unwrap_or_default();
    let known: Option<(&str, Option<&'static str>)> = match domain.as_str() {
        "gmail.com" | "googlemail.com" => Some((
            "imap.gmail.com",
            Some("Gmail takes an app password, made at myaccount.google.com/apppasswords"),
        )),
        "icloud.com" | "me.com" | "mac.com" => Some((
            "imap.mail.me.com",
            Some("iCloud takes an app-specific password, made at account.apple.com"),
        )),
        "outlook.com" | "hotmail.com" | "live.com" | "msn.com" => {
            Some((crate::oauth::IMAP_HOST, Some(MICROSOFT_NOTE)))
        }
        "yahoo.com" | "ymail.com" => Some((
            "imap.mail.yahoo.com",
            Some("Yahoo takes an app password, made in its account security settings"),
        )),
        "fastmail.com" | "fastmail.fm" => Some((
            "imap.fastmail.com",
            Some("Fastmail takes an app password, made in its settings"),
        )),
        _ => None,
    };
    match known {
        Some((host, note)) => (host.to_string(), note),
        None if domain.is_empty() => (String::new(), None),
        None => (format!("imap.{domain}"), None),
    }
}

/// `account` as a table of the file, what is as it would be without
/// saying left out.
fn table(a: &Account) -> toml_edit::Table {
    let mut t = toml_edit::Table::new();
    t["name"] = toml_edit::value(&a.name);
    t["email"] = toml_edit::value(&a.email);
    t["host"] = toml_edit::value(&a.host);
    if let Some(p) = a.port {
        t["port"] = toml_edit::value(i64::from(p));
    }
    if let Some(u) = a.user.as_ref().filter(|u| **u != a.email) {
        t["user"] = toml_edit::value(u);
    }
    if a.auth == Auth::Microsoft {
        t["auth"] = toml_edit::value("microsoft");
    }
    if let Some(v) = &a.tenant {
        t["tenant"] = toml_edit::value(v);
    }
    if let Some(v) = &a.client_id {
        t["client_id"] = toml_edit::value(v);
    }
    if a.keep == Keeper::Vault && a.password_command.is_none() {
        t["keep"] = toml_edit::value("vault");
    }
    match a.security {
        Security::Tls => {}
        Security::Starttls => t["security"] = toml_edit::value("starttls"),
        Security::None => t["security"] = toml_edit::value("none"),
    }
    if let Some(c) = &a.password_command {
        t["password_command"] = toml_edit::value(c);
    }
    t
}

fn read_doc(path: &Path) -> anyhow::Result<toml_edit::DocumentMut> {
    match std::fs::read_to_string(path) {
        Ok(text) => text
            .parse()
            .with_context(|| format!("cannot read {}", path.display())),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(Default::default()),
        Err(e) => Err(e).with_context(|| format!("cannot read {}", path.display())),
    }
}

/// Write `doc` to `path` whole, or not at all: to a file beside it first.
fn write_doc(path: &Path, doc: &toml_edit::DocumentMut) -> anyhow::Result<()> {
    if let Some(dir) = path.parent() {
        std::fs::create_dir_all(dir)?;
    }
    let tmp = path.with_extension("toml.new");
    std::fs::write(&tmp, doc.to_string())
        .with_context(|| format!("cannot write {}", tmp.display()))?;
    std::fs::rename(&tmp, path).with_context(|| format!("cannot write {}", path.display()))
}

/// Add `account` to the file at `path`, made if there is none, its other
/// accounts and comments kept as they are.
pub fn add(path: &Path, account: &Account) -> anyhow::Result<()> {
    if account.name.trim().is_empty() {
        bail!("an account needs a name");
    }
    if load(path)?
        .iter()
        .any(|a| a.name.eq_ignore_ascii_case(&account.name))
    {
        bail!("there is an account named {} already", account.name);
    }
    let mut doc = read_doc(path)?;
    let accounts = doc
        .entry("account")
        .or_insert_with(|| toml_edit::Item::ArrayOfTables(Default::default()))
        .as_array_of_tables_mut()
        .context("`account` in the file is not a list of [[account]] tables")?;
    accounts.push(table(account));
    write_doc(path, &doc)
}

/// Take the account named `name` out of the file at `path`; the account,
/// as it was.
pub fn remove(path: &Path, name: &str) -> anyhow::Result<Account> {
    let account = load(path)?
        .into_iter()
        .find(|a| a.name.eq_ignore_ascii_case(name))
        .ok_or_else(|| anyhow::anyhow!("no account named {name}"))?;
    let mut doc = read_doc(path)?;
    let named = |t: &toml_edit::Table| {
        t.get("name")
            .and_then(toml_edit::Item::as_str)
            .is_some_and(|n| n.eq_ignore_ascii_case(name))
    };
    let mut header = None;
    if let Some(accounts) = doc
        .get_mut("account")
        .and_then(toml_edit::Item::as_array_of_tables_mut)
    {
        // The comments above the first account are the file's, as its
        // heading: they stay, above the account after it, or at the end.
        if accounts.get(0).is_some_and(named) {
            header = accounts
                .get(0)
                .and_then(|t| t.decor().prefix())
                .and_then(|p| p.as_str())
                .map(str::to_string);
        }
        accounts.retain(|t| !named(t));
        if let Some(h) = &header
            && let Some(first) = accounts.get_mut(0)
        {
            let own = first
                .decor()
                .prefix()
                .and_then(|p| p.as_str())
                .unwrap_or_default()
                .to_string();
            first.decor_mut().set_prefix(format!("{h}{own}"));
            header = None;
        }
    }
    if let Some(h) = header {
        doc.set_trailing(h);
    }
    write_doc(path, &doc)?;
    Ok(account)
}

/// What `letterbox accounts` shows with none set up.
pub const EXAMPLE: &str = r#"[[account]]
name = "Work"
email = "me@example.com"
host = "imap.example.com"
# port = 993, or 143 with security = "starttls"
# user = "me", if it is not the address
#
# The password is kept by letterbox, as `letterbox accounts add` or A in
# letterbox keeps it: in the system's store, or with keep = "vault" in its
# vault. Or it is printed by a command of yours:
# password_command = "pass show mail/work"
"#;
