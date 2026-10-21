//! Adding an account, and taking one away, alike from the TUI and the
//! shell. An account is added only once it has signed in, so a mistyped
//! server or password is said at once, not found later; its password is
//! then kept where the system keeps passwords, and the account written to
//! the file without it.

use crate::config::{self, Account};
use crate::imap::Imap;
use crate::mail::Folder;
use crate::secret::{self, Keeper};
use std::path::Path;

/// Keep a password: given where, the host, the user and the password.
pub type Keep = fn(Keeper, &str, &str, &str) -> anyhow::Result<()>;

/// Sign in as `account`, with the password it was given as typed, or the
/// token Microsoft gave; if it can, keep that secret in `keeper`, by
/// `keep`, and add the account to the file at `path`. The store signed in,
/// and its folders.
pub fn add(
    path: &Path,
    mut account: Account,
    keeper: Keeper,
    keep: Keep,
) -> anyhow::Result<(Imap, Vec<Folder>)> {
    if config::load(path)?
        .iter()
        .any(|a| a.name.eq_ignore_ascii_case(&account.name))
    {
        anyhow::bail!("there is an account named {} already", account.name);
    }
    if account.password_command.is_none() {
        account.keep = keeper;
    }
    let mut imap = Imap::new(account.clone());
    let folders = crate::store::Store::folders(&mut imap)?;
    if let Some(secret) = account.password.take()
        && account.password_command.is_none()
    {
        keep(keeper, &account.host, account.user(), &secret)?;
    }
    config::add(path, &account)?;
    Ok((imap, folders))
}

/// Take the account named `name` out of the file at `path`, and forget
/// the password letterbox kept for it, unless `keep_password`; the account
/// as it was, and whether its password is forgotten, which it is not if it
/// is in a vault still locked.
pub fn remove(path: &Path, name: &str, keep_password: bool) -> anyhow::Result<(Account, bool)> {
    let account = config::remove(path, name)?;
    let forgotten = !keep_password && secret::forget(&account);
    Ok((account, forgotten))
}

/// An account from what is typed into the form or asked at the shell:
/// the server's security by its port, TLS but on 143, or on this machine.
pub fn account(name: &str, email: &str, host: &str, port: u16, user: &str) -> Account {
    let security = match port {
        993 => config::Security::Tls,
        143 => config::Security::Starttls,
        _ if config::local(host) => config::Security::None,
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
        auth: config::Auth::Password,
        tenant: None,
        client_id: None,
        keep: Keeper::default(),
        password_command: None,
        password: None,
        missing: None,
        smtp_host: None,
        smtp_port: None,
        trash: None,
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
