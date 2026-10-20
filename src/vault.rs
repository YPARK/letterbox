//! The vault: the passwords of accounts that keep them here, in one file
//! encrypted with a passphrase, in the age format (age-encryption.org),
//! by the age crate, in Rust. It is the same file on every system, and
//! `age -d` or `rage -d` opens it too, should letterbox not. The passphrase
//! is asked for once a run, and the passwords then kept in memory.

use age::secrecy::SecretString;
use anyhow::{Context, bail};
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::sync::Mutex;

struct Open {
    path: PathBuf,
    passphrase: SecretString,
    /// Each password, by `user@host`.
    entries: BTreeMap<String, String>,
}

static OPEN: Mutex<Option<Open>> = Mutex::new(None);

fn open() -> std::sync::MutexGuard<'static, Option<Open>> {
    OPEN.lock().unwrap_or_else(|e| e.into_inner())
}

/// `$LETTERBOX_VAULT`, else `$XDG_DATA_HOME/letterbox/vault.age`, else
/// `~/.local/share/letterbox/vault.age`.
pub fn path() -> PathBuf {
    if let Some(p) = std::env::var_os("LETTERBOX_VAULT").filter(|p| !p.is_empty()) {
        return PathBuf::from(p);
    }
    std::env::var_os("XDG_DATA_HOME")
        .filter(|d| !d.is_empty())
        .map(PathBuf::from)
        .unwrap_or_else(|| {
            PathBuf::from(std::env::var_os("HOME").unwrap_or_default()).join(".local/share")
        })
        .join("letterbox/vault.age")
}

pub fn exists() -> bool {
    path().is_file()
}

pub fn is_open() -> bool {
    open().is_some()
}

fn key(host: &str, user: &str) -> String {
    format!("{user}@{host}")
}

/// Open the vault with `passphrase`; a new, empty one if there is none
/// yet, to be written once a password is put in it.
pub fn unlock(passphrase: &str) -> anyhow::Result<()> {
    let path = path();
    let entries = match std::fs::read(&path) {
        Ok(bytes) => {
            let identity = age::scrypt::Identity::new(SecretString::from(passphrase.to_string()));
            let plain = age::decrypt(&identity, &bytes).map_err(|e| match e {
                age::DecryptError::DecryptionFailed | age::DecryptError::NoMatchingKeys => {
                    anyhow::anyhow!("that is not the vault's passphrase")
                }
                e => anyhow::anyhow!("cannot open the vault {}: {e}", path.display()),
            })?;
            serde_json::from_slice(&plain).context("the vault holds what letterbox cannot read")?
        }
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
            if passphrase.chars().count() < 8 {
                bail!("a vault's passphrase is 8 characters or more");
            }
            BTreeMap::new()
        }
        Err(e) => return Err(e).with_context(|| format!("cannot read {}", path.display())),
    };
    *open() = Some(Open {
        path,
        passphrase: SecretString::from(passphrase.to_string()),
        entries,
    });
    Ok(())
}

pub fn get(host: &str, user: &str) -> anyhow::Result<String> {
    let guard = open();
    let Some(v) = guard.as_ref() else {
        bail!("the vault is locked: letterbox asks for its passphrase as it starts");
    };
    v.entries
        .get(&key(host, user))
        .cloned()
        .ok_or_else(|| anyhow::anyhow!("no password for {user} at {host} in the vault"))
}

pub fn put(host: &str, user: &str, password: &str) -> anyhow::Result<()> {
    let mut guard = open();
    let Some(v) = guard.as_mut() else {
        bail!("the vault is locked");
    };
    v.entries.insert(key(host, user), password.to_string());
    write(v)
}

/// Take the password for `user` at `host` out of the vault; whether it
/// could be, the vault being open.
pub fn remove(host: &str, user: &str) -> anyhow::Result<bool> {
    let mut guard = open();
    let Some(v) = guard.as_mut() else {
        return Ok(false);
    };
    if v.entries.remove(&key(host, user)).is_some() {
        write(v)?;
    }
    Ok(true)
}

/// Write the vault whole, or not at all: to a file beside it first, which
/// only you may read.
fn write(v: &Open) -> anyhow::Result<()> {
    let plain = serde_json::to_vec(&v.entries)?;
    let mut recipient = age::scrypt::Recipient::new(v.passphrase.clone());
    if cfg!(test) {
        recipient.set_work_factor(10);
    }
    let sealed = age::encrypt(&recipient, &plain).context("cannot encrypt the vault")?;
    let dir = v.path.parent().unwrap_or(Path::new("."));
    std::fs::create_dir_all(dir)?;
    let tmp = v.path.with_extension("age.new");
    write_private(&tmp, &sealed)?;
    std::fs::rename(&tmp, &v.path).with_context(|| format!("cannot write {}", v.path.display()))
}

fn write_private(path: &Path, bytes: &[u8]) -> anyhow::Result<()> {
    use std::io::Write;
    let mut options = std::fs::OpenOptions::new();
    options.write(true).create(true).truncate(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.mode(0o600);
    }
    let mut file = options
        .open(path)
        .with_context(|| format!("cannot write {}", path.display()))?;
    file.write_all(bytes)?;
    file.sync_all()?;
    Ok(())
}

/// Lock the vault again: for tests.
#[cfg(test)]
pub fn lock() {
    *open() = None;
}
