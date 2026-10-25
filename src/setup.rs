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
/// Find a password: given where, the host and the user.
pub type Find = fn(Keeper, &str, &str) -> anyhow::Result<String>;
/// Forget a password: given where, the host and the user.
pub type Forget = fn(Keeper, &str, &str) -> anyhow::Result<()>;

/// Move every password letterbox keeps for the accounts in the file at
/// `path`, and the map's key in `map`, to `to`: each read where it is by
/// `find`, kept in `to` by `keep`, its account or `map.toml` made to say
/// so, and only then forgotten where it was by `forget`, so that none is
/// lost if a step fails. What was done, a line each.
pub fn move_secrets(
    path: &Path,
    map: &Path,
    to: Keeper,
    find: Find,
    keep: Keep,
    forget: Forget,
) -> anyhow::Result<Vec<String>> {
    use anyhow::Context;
    let mut said = Vec::new();
    let stays = |what: &str, why: String| format!("{what}: moved, but its old copy stays: {why}");
    for a in config::load(path)? {
        if a.password_command.is_some() {
            said.push(format!("{}: its password_command is left as it is", a.name));
            continue;
        }
        if a.keep == to {
            continue;
        }
        let (host, user) = (a.host.as_str(), a.user());
        let secret = find(a.keep, host, user)
            .with_context(|| format!("cannot read {}'s password", a.name))?;
        keep(to, host, user, &secret)
            .with_context(|| format!("cannot keep {}'s password", a.name))?;
        config::set_keep(path, &a.name, to)?;
        said.push(match forget(a.keep, host, user) {
            Ok(()) => format!("{}: kept {}", a.name, to.about()),
            Err(e) => stays(&a.name, format!("{e:#}")),
        });
    }
    match crate::index::key::move_to(map, to, find, keep, forget)? {
        (true, None) => said.push(format!("the map's key: kept {}", to.about())),
        (true, Some(why)) => said.push(stays("the map's key", why)),
        (false, _) => {}
    }
    Ok(said)
}

/// Sign in as `account`, with the password it was given as typed (an app
/// password as its provider shows it, run together), or the token
/// Microsoft gave; if it can, keep that secret in `keeper`, by
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
    if account.auth == config::Auth::Password {
        account.password = account
            .password
            .map(|typed| config::app_password(&account.host, &typed));
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
/// `example.edu`.
pub fn name_for(email: &str) -> String {
    email
        .rsplit_once('@')
        .map(|(_, d)| d.trim().to_lowercase())
        .unwrap_or_default()
}
