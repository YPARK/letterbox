//! The accounts, in a TOML file of `[[account]]` tables, and their
//! passwords: from a command given, else, on macOS, from the keychain.

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
    /// A shell command that prints the password, as `pass show mail/work`.
    pub password_command: Option<String>,
    /// The password as typed, while an account is being added: never
    /// written to the file.
    #[serde(skip)]
    pub password: Option<String>,
}

#[derive(Debug, Default, Deserialize)]
#[serde(deny_unknown_fields)]
struct File {
    #[serde(default)]
    account: Vec<Account>,
}

impl Account {
    pub fn user(&self) -> &str {
        self.user.as_deref().unwrap_or(&self.email)
    }

    pub fn port(&self) -> u16 {
        self.port.unwrap_or(match self.security {
            Security::Tls => 993,
            Security::Starttls | Security::None => 143,
        })
    }

    /// The password: printed by `password_command`, or, on macOS, kept in
    /// the keychain for the host and user.
    pub fn password(&self) -> anyhow::Result<String> {
        if let Some(p) = &self.password {
            return Ok(p.clone());
        }
        if let Some(command) = &self.password_command {
            let out = Command::new("sh")
                .args(["-c", command])
                .output()
                .with_context(|| format!("cannot run {command}"))?;
            if !out.status.success() {
                bail!(
                    "{command} gave no password: {}",
                    String::from_utf8_lossy(&out.stderr).trim()
                );
            }
            return Ok(first_line(&out.stdout));
        }
        if cfg!(target_os = "macos") {
            let out = Command::new("security")
                .args(["find-internet-password", "-s", &self.host])
                .args(["-a", self.user(), "-w"])
                .output();
            if let Ok(out) = out
                && out.status.success()
            {
                return Ok(first_line(&out.stdout));
            }
            bail!(
                "no password for {} at {} in the keychain: add it with\n  \
                 security add-internet-password -s {} -a {} -w\n\
                 or give the account a password_command",
                self.user(),
                self.host,
                self.host,
                self.user()
            );
        }
        bail!(
            "no password for {} at {}: give the account a password_command",
            self.user(),
            self.host
        )
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
        "outlook.com" | "hotmail.com" | "live.com" | "msn.com" => Some((
            "outlook.office365.com",
            Some("Outlook signs in through the browser, which letterbox cannot do yet"),
        )),
        "yahoo.com" | "ymail.com" => Some((
            "imap.mail.yahoo.com",
            Some("Yahoo takes an app password, made in its account security settings"),
        )),
        "fastmail.com" | "fastmail.fm" => Some((
            "imap.fastmail.com",
            Some("Fastmail takes an app password, made in its settings"),
        )),
        "proton.me" | "protonmail.com" => Some((
            "127.0.0.1",
            Some("Proton is read through its Bridge: port 1143, security \"starttls\""),
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
# The password is the macOS keychain's, put there with
#   security add-internet-password -s imap.example.com -a me@example.com -w
# or printed by a command:
# password_command = "pass show mail/work"
"#;
