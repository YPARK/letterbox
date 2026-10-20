//! Adding an account, and taking one away, alike from the TUI and the
//! shell. An account is added only once it has signed in, so a mistyped
//! server or password is said at once, not found later; its password is
//! then kept where the system keeps passwords, and the account written to
//! the file without it.

use crate::config::{self, Account};
use crate::imap::Imap;
use crate::mail::Folder;
use crate::secret;
use std::path::Path;

/// Where a password is kept: given the host, the user and the password,
/// the `password_command` that finds it again, if one is needed.
pub type Keep = fn(&str, &str, &str) -> anyhow::Result<Option<String>>;

/// Sign in as `account`, with the password it was given as typed; if it
/// can, keep that password with `keep`, and add the account to the file
/// at `path`. The store signed in, and its folders.
pub fn add(path: &Path, mut account: Account, keep: Keep) -> anyhow::Result<(Imap, Vec<Folder>)> {
    if config::load(path)?
        .iter()
        .any(|a| a.name.eq_ignore_ascii_case(&account.name))
    {
        anyhow::bail!("there is an account named {} already", account.name);
    }
    let mut imap = Imap::new(account.clone());
    let folders = crate::store::Store::folders(&mut imap)?;
    if let Some(password) = account.password.take()
        && account.password_command.is_none()
    {
        account.password_command = keep(&account.host, account.user(), &password)?;
    }
    config::add(path, &account)?;
    Ok((imap, folders))
}

/// Take the account named `name` out of the file at `path`, and forget
/// the password letterbox kept for it, unless `keep_password`.
pub fn remove(path: &Path, name: &str, keep_password: bool) -> anyhow::Result<Account> {
    let account = config::remove(path, name)?;
    let kept_here = account
        .password_command
        .as_deref()
        .is_none_or(|c| c.starts_with("secret-tool lookup service letterbox "));
    if kept_here && !keep_password {
        secret::forget(&account.host, account.user());
    }
    Ok(account)
}

/// An account from what is typed into the form or asked at the shell:
/// the server's security by its port, TLS but on 143, or on this machine.
pub fn account(name: &str, email: &str, host: &str, port: u16, user: &str) -> Account {
    let security = match port {
        993 => config::Security::Tls,
        143 => config::Security::Starttls,
        _ if matches!(host, "localhost" | "127.0.0.1" | "::1") => config::Security::None,
        _ => config::Security::Tls,
    };
    let email = email.trim().to_string();
    let user = user.trim();
    Account {
        name: name.trim().to_string(),
        host: host.trim().to_string(),
        port: (port != 993).then_some(port),
        user: (!user.is_empty() && user != email).then(|| user.to_string()),
        email,
        security,
        password_command: None,
        password: None,
    }
}

/// The name an account is given by default: its address's domain, as
/// `ubc.ca`.
pub fn name_for(email: &str) -> String {
    email
        .rsplit_once('@')
        .map(|(_, d)| d.trim().to_lowercase())
        .unwrap_or_default()
}
