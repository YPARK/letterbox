//! Passwords kept where the system keeps them, never in the accounts file:
//! on macOS, the keychain, as an internet password for the host and user,
//! which an account without a `password_command` is looked up in; on
//! Linux, the Secret Service, through `secret-tool`, which an account is
//! then given a `password_command` to look up. Either is given the password
//! on its input, not on its command line, where other programs would see it.

use anyhow::{Context, bail};
use std::io::Write;
use std::process::{Command, Stdio};

/// Quoted for `security -i`, which reads `\` and `"` escaped in quotes.
fn quoted(s: &str) -> String {
    format!("\"{}\"", s.replace('\\', "\\\\").replace('"', "\\\""))
}

fn run(program: &str, args: &[&str], input: &str) -> anyhow::Result<()> {
    let mut child = Command::new(program)
        .args(args)
        .stdin(Stdio::piped())
        .stdout(Stdio::null())
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
    Ok(())
}

fn secret_tool() -> bool {
    Command::new("secret-tool")
        .arg("--version")
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status()
        .is_ok()
}

/// Keep `password` for `user` at `host`; the `password_command` that finds
/// it again, if the account needs one to.
pub fn store(host: &str, user: &str, password: &str) -> anyhow::Result<Option<String>> {
    if password.contains(['\n', '\r']) {
        bail!("a password cannot hold a line break");
    }
    if cfg!(target_os = "macos") {
        let line = format!(
            "add-internet-password -U -s {} -a {} -l {} -w {}\n",
            quoted(host),
            quoted(user),
            quoted(&format!("letterbox: {user}")),
            quoted(password)
        );
        run("security", &["-i"], &line).context("cannot keep the password in the keychain")?;
        return Ok(None);
    }
    if secret_tool() {
        let label = format!("--label=letterbox: {user} at {host}");
        let attributes = ["service", "letterbox", "host", host, "user", user];
        let mut args = vec!["store", label.as_str()];
        args.extend(attributes);
        run("secret-tool", &args, password).context("cannot keep the password")?;
        return Ok(Some(format!(
            "secret-tool lookup service letterbox host {} user {}",
            shell_word(host),
            shell_word(user)
        )));
    }
    bail!(
        "there is nowhere here to keep a password: install secret-tool, or give the \
         account a password_command"
    )
}

/// Forget the password kept for `user` at `host`, if one is.
pub fn forget(host: &str, user: &str) {
    if cfg!(target_os = "macos") {
        let _ = Command::new("security")
            .args(["delete-internet-password", "-s", host, "-a", user])
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .status();
    } else if secret_tool() {
        let _ = Command::new("secret-tool")
            .args(["clear", "service", "letterbox", "host", host, "user", user])
            .status();
    }
}

/// `s` quoted for `sh`, where it needs to be.
fn shell_word(s: &str) -> String {
    if s.chars()
        .all(|c| c.is_ascii_alphanumeric() || "@._-+".contains(c))
    {
        s.to_string()
    } else {
        format!("'{}'", s.replace('\'', "'\\''"))
    }
}
