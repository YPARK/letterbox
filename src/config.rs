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
