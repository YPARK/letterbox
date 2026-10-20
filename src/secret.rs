//! Passwords kept encrypted, never in the accounts file: in the macOS
//! keychain, as an internet password for the host and user, which an
//! account without a `password_command` is looked up in; on Linux in the
//! Secret Service, through `secret-tool`; or anywhere in a file encrypted
//! with gpg to your own key, as `pass` keeps them. The latter two give the
//! account a `password_command` to find the password again. Each is given
//! the password on its input, not on its command line, where other programs
//! would see it.

use anyhow::{Context, bail};
use std::io::Write;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

/// Where a password is kept.
#[derive(Clone, Copy, Debug, PartialEq, Eq, clap::ValueEnum)]
pub enum Keeper {
    /// The macOS keychain.
    Keychain,
    /// The Secret Service, as GNOME's keyring or KWallet, by `secret-tool`.
    Keyring,
    /// A file encrypted with gpg to your own key.
    Gpg,
}

impl Keeper {
    pub fn name(self) -> &'static str {
        match self {
            Keeper::Keychain => "keychain",
            Keeper::Keyring => "keyring",
            Keeper::Gpg => "gpg",
        }
    }
}

/// The keepers there are here, the system's first.
pub fn keepers() -> Vec<Keeper> {
    let mut all = Vec::new();
    if cfg!(target_os = "macos") {
        all.push(Keeper::Keychain);
    } else if found("secret-tool") {
        all.push(Keeper::Keyring);
    }
    if found("gpg") {
        all.push(Keeper::Gpg);
    }
    all
}

fn found(program: &str) -> bool {
    Command::new(program)
        .arg("--version")
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status()
        .is_ok_and(|s| s.success())
}

/// Quoted for `security -i`, which reads `\` and `"` escaped in quotes.
fn quoted(s: &str) -> String {
    format!("\"{}\"", s.replace('\\', "\\\\").replace('"', "\\\""))
}

fn run(program: &str, args: &[&str], input: &str) -> anyhow::Result<String> {
    let mut child = Command::new(program)
        .args(args)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .with_context(|| format!("cannot run {program}"))?;
    child
        .stdin
        .take()
        .expect("piped")
        .write_all(input.as_bytes())?;
    let out = child.wait_with_output()?;
    if !out.status.success() {
        bail!("{program}: {}", String::from_utf8_lossy(&out.stderr).trim());
    }
    Ok(String::from_utf8_lossy(&out.stdout).into_owned())
}

/// Keep `password` for `user` at `host` with `keeper`; the
/// `password_command` that finds it again, if the account needs one to.
pub fn store(
    keeper: Keeper,
    host: &str,
    user: &str,
    password: &str,
) -> anyhow::Result<Option<String>> {
    if password.contains(['\n', '\r']) {
        bail!("a password cannot hold a line break");
    }
    match keeper {
        Keeper::Keychain => {
            let line = format!(
                "add-internet-password -U -s {} -a {} -l {} -w {}\n",
                quoted(host),
                quoted(user),
                quoted(&format!("letterbox: {user}")),
                quoted(password)
            );
            run("security", &["-i"], &line).context("cannot keep the password in the keychain")?;
            Ok(None)
        }
        Keeper::Keyring => {
            let label = format!("--label=letterbox: {user} at {host}");
            let mut args = vec!["store", label.as_str()];
            args.extend(["service", "letterbox", "host", host, "user", user]);
            run("secret-tool", &args, password).context("cannot keep the password")?;
            Ok(Some(format!(
                "secret-tool lookup service letterbox host {} user {}",
                shell_word(host),
                shell_word(user)
            )))
        }
        Keeper::Gpg => {
            let file = gpg_file(host, user);
            gpg_encrypt(&file, password)?;
            Ok(Some(format!(
                "gpg --quiet --decrypt {}",
                shell_word(&file.to_string_lossy())
            )))
        }
    }
}

/// Where the passwords encrypted with gpg are: `$XDG_DATA_HOME/letterbox/
/// passwords`, else `~/.local/share/letterbox/passwords`.
pub fn gpg_dir() -> PathBuf {
    std::env::var_os("XDG_DATA_HOME")
        .filter(|d| !d.is_empty())
        .map(PathBuf::from)
        .unwrap_or_else(|| {
            PathBuf::from(std::env::var_os("HOME").unwrap_or_default()).join(".local/share")
        })
        .join("letterbox/passwords")
}

fn gpg_file(host: &str, user: &str) -> PathBuf {
    let name: String = format!("{user}@{host}")
        .chars()
        .map(|c| {
            if c.is_ascii_alphanumeric() || "@._-+".contains(c) {
                c
            } else {
                '_'
            }
        })
        .collect();
    gpg_dir().join(format!("{name}.gpg"))
}

/// The fingerprint of the first secret key gpg has: yours.
fn gpg_key() -> anyhow::Result<String> {
    let out = Command::new("gpg")
        .args(["--list-secret-keys", "--with-colons"])
        .output()
        .context("cannot run gpg")?;
    let text = String::from_utf8_lossy(&out.stdout);
    let mut lines = text.lines().skip_while(|l| !l.starts_with("sec:"));
    lines
        .find_map(|l| l.strip_prefix("fpr:"))
        .and_then(|l| l.split(':').find(|f| !f.is_empty()))
        .map(str::to_string)
        .ok_or_else(|| {
            anyhow::anyhow!(
                "gpg has no key of yours to encrypt to: make one with\n  \
                 gpg --quick-generate-key 'Your Name <you@example.com>'"
            )
        })
}

fn gpg_encrypt(file: &Path, password: &str) -> anyhow::Result<()> {
    let dir = file.parent().expect("in a folder");
    std::fs::create_dir_all(dir)?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(dir, std::fs::Permissions::from_mode(0o700))?;
    }
    let key = gpg_key()?;
    let out = file.to_string_lossy();
    run(
        "gpg",
        &[
            "--batch",
            "--yes",
            "--quiet",
            "--trust-model",
            "always",
            "--encrypt",
            "--recipient",
            &key,
            "--output",
            &out,
        ],
        password,
    )
    .context("cannot encrypt the password with gpg")?;
    Ok(())
}

/// Forget the password kept for an account at `host` as `user`, found by
/// `command`, if letterbox kept it.
pub fn forget(host: &str, user: &str, command: Option<&str>) {
    match command {
        None if cfg!(target_os = "macos") => {
            let _ = Command::new("security")
                .args(["delete-internet-password", "-s", host, "-a", user])
                .stdout(Stdio::null())
                .stderr(Stdio::null())
                .status();
        }
        Some(c) if c.starts_with("secret-tool lookup service letterbox ") => {
            let _ = Command::new("secret-tool")
                .args(["clear", "service", "letterbox", "host", host, "user", user])
                .status();
        }
        Some(c) if c.starts_with("gpg --quiet --decrypt ") => {
            let file = gpg_file(host, user);
            if c.contains(&*file.to_string_lossy()) {
                let _ = std::fs::remove_file(file);
            }
        }
        _ => {}
    }
}

/// Whether finding a password with `command` may ask for a passphrase on
/// the terminal, as gpg's pinentry does: to be run before the TUI takes it.
pub fn asks(command: &str) -> bool {
    command.split_whitespace().next().is_some_and(|p| {
        p.ends_with("gpg") || p.ends_with("gpg2") || p == "pass" || p.ends_with("/pass")
    })
}

/// `s` quoted for `sh`, where it needs to be.
fn shell_word(s: &str) -> String {
    if s.chars()
        .all(|c| c.is_ascii_alphanumeric() || "@._-+/".contains(c))
    {
        s.to_string()
    } else {
        format!("'{}'", s.replace('\'', "'\\''"))
    }
}
