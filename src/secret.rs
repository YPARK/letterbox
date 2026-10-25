//! Passwords kept encrypted, never in the accounts file, by Rust in
//! letterbox itself, with no program run for them: in the system's own
//! store, by keyring-rs, which is the keychain on macOS, the Credential
//! Manager on Windows and the Secret Service on Linux; or in letterbox's
//! vault, a file encrypted with a passphrase, for a system without one, as
//! a server reached over ssh.

use crate::config::Account;
use crate::vault;
use anyhow::{Context, bail};
use serde::{Deserialize, Serialize};

/// Where a password is kept.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize, clap::ValueEnum)]
#[serde(rename_all = "lowercase")]
pub enum Keeper {
    /// The system's own store: the keychain, the Credential Manager, or
    /// the Secret Service.
    #[default]
    Keyring,
    /// letterbox's vault, encrypted with a passphrase.
    Vault,
}

impl Keeper {
    pub fn name(self) -> &'static str {
        match self {
            Keeper::Keyring => "keyring",
            Keeper::Vault => "vault",
        }
    }

    /// What it is, as it is here.
    pub fn about(self) -> &'static str {
        match self {
            Keeper::Keyring if cfg!(target_os = "macos") => "in the macOS keychain",
            Keeper::Keyring if cfg!(windows) => "in the Windows Credential Manager",
            Keeper::Keyring => "in the Secret Service, as GNOME's keyring or KWallet",
            Keeper::Vault => "in letterbox's vault, encrypted with a passphrase of yours",
        }
    }
}

/// Where a password may be kept here: the system's store first, where
/// there is one.
pub fn keepers() -> Vec<Keeper> {
    let mut all = Vec::new();
    if keyring::Entry::store_status().is_ok() {
        all.push(Keeper::Keyring);
    }
    all.push(Keeper::Vault);
    all
}

fn entry(host: &str, user: &str) -> anyhow::Result<keyring::Entry> {
    keyring::Entry::new(&format!("letterbox {host}"), user)
        .context("the system's password store cannot be reached")
}

/// Keep `password` for `user` at `host` in `keeper`.
pub fn store(keeper: Keeper, host: &str, user: &str, password: &str) -> anyhow::Result<()> {
    match keeper {
        Keeper::Keyring => entry(host, user)?
            .set_password(password)
            .context("cannot keep the password in the system's store"),
        Keeper::Vault => vault::put(host, user, password),
    }
}

/// The password kept for `user` at `host` in `keeper`.
pub fn find(keeper: Keeper, host: &str, user: &str) -> anyhow::Result<String> {
    match keeper {
        Keeper::Keyring => match entry(host, user)?.get_password() {
            Ok(p) => Ok(p),
            Err(keyring::Error::NoEntry) => bail!(
                "no password for {user} at {host} in the system's store: remove the account \
                 and add it again, or give it a password_command"
            ),
            Err(e) => Err(e).context("cannot read the system's password store"),
        },
        Keeper::Vault => vault::get(host, user),
    }
}

/// Forget the password kept for `user` at `host` in `keeper`; none kept
/// there is forgotten already.
pub fn remove(keeper: Keeper, host: &str, user: &str) -> anyhow::Result<()> {
    match keeper {
        Keeper::Keyring => match entry(host, user)?.delete_credential() {
            Ok(()) | Err(keyring::Error::NoEntry) => Ok(()),
            Err(e) => Err(e).context("cannot forget it in the system's store"),
        },
        Keeper::Vault => vault::remove(host, user).map(|_| ()),
    }
}

/// Forget the password kept for `account`, if letterbox kept it; whether
/// it is gone, or, kept in a vault still locked, is not.
pub fn forget(account: &Account) -> bool {
    if account.password_command.is_some() {
        return true;
    }
    let (host, user) = (&account.host, account.user());
    match account.keep {
        Keeper::Keyring => {
            if let Ok(e) = entry(host, user) {
                let _ = e.delete_credential();
            }
            true
        }
        Keeper::Vault => vault::remove(host, user).unwrap_or(false),
    }
}
