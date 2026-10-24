//! Sending a message: SMTP submission, as RFC 6409, on the same rustls
//! stack as IMAP. Port 587 starts in the clear and asks for TLS; port 465
//! is TLS from the greeting. A server that does not advertise `8BITMIME`
//! is sent the body as quoted-printable.

use crate::config::{Account, Auth, Security};
use crate::imap::{self, Stream};
use crate::mail;
use crate::oauth;
use anyhow::{Context, bail};
use base64::Engine;
use std::io::{Read, Write};

/// Whether `host` files a copy of what is sent, so letterbox does not
/// append one to Sent as well. Gmail's submission server does.
pub fn files_sent(host: &str) -> bool {
    host.eq_ignore_ascii_case("smtp.gmail.com")
}

/// Send `raw` as `account`. The bytes are what was accepted: rewritten as
/// quoted-printable when the server has no `8BITMIME` and the body is not
/// ASCII, so the copy kept in Sent is the one the recipient was sent.
pub fn submit(account: &Account, raw: &[u8]) -> anyhow::Result<Vec<u8>> {
    let (host, port) = account.submission();
    if host.is_empty() {
        bail!("no server to send through: set smtp_host for this account");
    }
    if account.security == Security::None && !crate::config::local(&host) {
        bail!(
            "{host}: security = \"none\" sends the password unencrypted; it is for a server \
             on this machine only"
        );
    }
    let implicit = port == 465 && account.security != Security::None;
    let tcp = imap::dial(&host, port)?;
    if implicit {
        let mut stream = imap::tls(&host, tcp)?;
        converse(&mut stream, account, raw)
    } else if account.security == Security::None {
        let mut stream = Stream::Plain(tcp);
        converse(&mut stream, account, raw)
    } else {
        let mut stream = Stream::Plain(tcp);
        greet(&mut stream)?;
        ehlo(&mut stream)?;
        command(&mut stream, "STARTTLS")?;
        let Stream::Plain(tcp) = stream else {
            bail!("already encrypted");
        };
        let mut stream = imap::tls(&host, tcp)?;
        converse_after_tls(&mut stream, account, raw)
    }
}

fn converse(stream: &mut Stream, account: &Account, raw: &[u8]) -> anyhow::Result<Vec<u8>> {
    greet(stream)?;
    converse_after_tls(stream, account, raw)
}

fn converse_after_tls(
    stream: &mut Stream,
    account: &Account,
    raw: &[u8],
) -> anyhow::Result<Vec<u8>> {
    let hello = ehlo(stream)?;
    authenticate(stream, account)?;
    let raw = for_transfer(raw, hello.contains("8BITMIME"));
    let from = account.email.trim();
    command(stream, &format!("MAIL FROM:<{from}>"))?;
    let recipients = recipients(&raw)?;
    if recipients.is_empty() {
        bail!("say who it is to");
    }
    for to in &recipients {
        command(stream, &format!("RCPT TO:<{to}>"))?;
    }
    let (code, text) = say(stream, "DATA")?;
    if code != 354 {
        bail!("the server would not take the message: {}", first(&text));
    }
    write_data(stream, &raw)?;
    let (code, text) = read_reply(stream)?;
    if !(200..300).contains(&code) {
        bail!("the server did not take the message: {}", first(&text));
    }
    let _ = say(stream, "QUIT");
    Ok(raw)
}

fn greet(stream: &mut Stream) -> anyhow::Result<()> {
    let (code, text) = read_reply(stream)?;
    if !(200..300).contains(&code) {
        bail!("the server greeted letterbox with {}", first(&text));
    }
    Ok(())
}

/// What `EHLO` answered, so `8BITMIME` can be seen in it.
fn ehlo(stream: &mut Stream) -> anyhow::Result<String> {
    let (code, text) = say(stream, "EHLO letterbox")?;
    if !(200..300).contains(&code) {
        bail!("the server refused EHLO: {}", first(&text));
    }
    Ok(text)
}

fn authenticate(stream: &mut Stream, account: &Account) -> anyhow::Result<()> {
    let initial = match account.auth {
        Auth::Password => {
            let secret = account.password()?;
            let raw = format!("\0{}\0{secret}", account.user());
            format!(
                "AUTH PLAIN {}",
                base64::engine::general_purpose::STANDARD.encode(raw.as_bytes())
            )
        }
        Auth::Microsoft => {
            let token = oauth::access(account)?;
            let raw = oauth::XOAuth2 {
                user: account.user().to_string(),
                token,
            }
            .secret();
            format!(
                "AUTH XOAUTH2 {}",
                base64::engine::general_purpose::STANDARD.encode(raw.as_bytes())
            )
        }
    };
    let (code, text) = say(stream, &initial)?;
    if (200..300).contains(&code) {
        return Ok(());
    }
    // A challenge, and then nothing: the initial response was the whole secret.
    if code == 334 {
        let (code, text) = say(stream, "")?;
        if (200..300).contains(&code) {
            return Ok(());
        }
        return refused(account, code, &text);
    }
    refused(account, code, &text)
}

fn refused(account: &Account, code: u16, text: &str) -> anyhow::Result<()> {
    if account.auth == Auth::Microsoft {
        bail!(
            "the server refused the token ({code} {}); sign in again to send",
            first(text)
        );
    }
    bail!("the server refused the password ({code} {})", first(text))
}

fn command(stream: &mut Stream, line: &str) -> anyhow::Result<()> {
    let (code, text) = say(stream, line)?;
    if (200..300).contains(&code) {
        return Ok(());
    }
    bail!("{line} was refused: {}", first(&text))
}

fn say(stream: &mut Stream, line: &str) -> anyhow::Result<(u16, String)> {
    stream.write_all(line.as_bytes())?;
    stream.write_all(b"\r\n")?;
    stream.flush()?;
    read_reply(stream)
}

fn read_reply(stream: &mut Stream) -> anyhow::Result<(u16, String)> {
    let mut text = String::new();
    loop {
        let line = read_line(stream)?;
        if line.len() < 3 || !line.as_bytes()[..3].iter().all(u8::is_ascii_digit) {
            bail!("the server answered {}", mail::clean(&line));
        }
        let code: u16 = line[..3].parse().unwrap_or(0);
        let more = line.as_bytes().get(3) == Some(&b'-');
        if !text.is_empty() {
            text.push('\n');
        }
        text.push_str(line.trim());
        if !more {
            return Ok((code, text));
        }
    }
}

fn read_line(stream: &mut Stream) -> anyhow::Result<String> {
    let mut buf = Vec::new();
    let mut byte = [0u8; 1];
    loop {
        let n = stream.read(&mut byte).context("the server hung up")?;
        if n == 0 {
            bail!("the server hung up");
        }
        buf.push(byte[0]);
        if byte[0] == b'\n' || buf.len() > 8192 {
            break;
        }
    }
    Ok(String::from_utf8_lossy(&buf)
        .trim_end_matches(['\r', '\n'])
        .to_string())
}

/// `raw` on the wire: a line that begins with `.` is sent with another, so
/// it is not the end of the message.
fn write_data(stream: &mut Stream, raw: &[u8]) -> anyhow::Result<()> {
    let text = String::from_utf8_lossy(raw);
    let normalized = text.replace("\r\n", "\n").replace('\r', "\n");
    let ended = normalized.ends_with('\n');
    let parts: Vec<&str> = normalized.split('\n').collect();
    let n = if ended {
        parts.len().saturating_sub(1)
    } else {
        parts.len()
    };
    for line in parts.iter().take(n) {
        if line.starts_with('.') {
            stream.write_all(b".")?;
        }
        stream.write_all(line.as_bytes())?;
        stream.write_all(b"\r\n")?;
    }
    stream.write_all(b".\r\n")?;
    stream.flush()?;
    Ok(())
}

/// `raw` as this server can take it. With `8BITMIME`, or an ASCII body, it
/// is unchanged; otherwise the body is quoted-printable.
pub fn for_transfer(raw: &[u8], eightbit: bool) -> Vec<u8> {
    if eightbit || raw.is_ascii() {
        return raw.to_vec();
    }
    let Some(split) = header_end(raw) else {
        return raw.to_vec();
    };
    let (head, body) = raw.split_at(split);
    let mut head = String::from_utf8_lossy(head).replace(
        "Content-Transfer-Encoding: 8bit",
        "Content-Transfer-Encoding: quoted-printable",
    );
    if !head
        .to_ascii_lowercase()
        .contains("content-transfer-encoding:")
        && let Some(stripped) = head.strip_suffix("\r\n\r\n")
    {
        head = format!("{stripped}\r\nContent-Transfer-Encoding: quoted-printable\r\n\r\n");
    }
    let encoded = quoted_printable(&String::from_utf8_lossy(body));
    let mut out = head.into_bytes();
    out.extend(encoded.into_bytes());
    out
}

fn header_end(raw: &[u8]) -> Option<usize> {
    raw.windows(4)
        .position(|w| w == b"\r\n\r\n")
        .map(|p| p + 4)
        .or_else(|| raw.windows(2).position(|w| w == b"\n\n").map(|p| p + 2))
}

fn quoted_printable(body: &str) -> String {
    let mut out = String::new();
    let mut col = 0;
    for &b in body.as_bytes() {
        if b == b'\n' {
            out.push_str("\r\n");
            col = 0;
            continue;
        }
        if b == b'\r' {
            continue;
        }
        let plain = matches!(b, 33..=60 | 62..=126);
        let piece = if plain {
            (b as char).to_string()
        } else {
            format!("={b:02X}")
        };
        if col + piece.len() > 75 {
            out.push_str("=\r\n");
            col = 0;
        }
        out.push_str(&piece);
        col += piece.len();
    }
    if !out.ends_with("\r\n") {
        out.push_str("\r\n");
    }
    out
}

fn recipients(raw: &[u8]) -> anyhow::Result<Vec<String>> {
    let mut out = Vec::new();
    for name in ["To", "Cc", "Bcc"] {
        let Some(value) = mail::field(raw, name) else {
            continue;
        };
        for part in crate::compose::split_addrs(&value) {
            let Some(spec) = crate::compose::addr_spec(&part) else {
                bail!("{part} is not an address");
            };
            if !out.contains(&spec) {
                out.push(spec);
            }
        }
    }
    Ok(out)
}

fn first(text: &str) -> String {
    mail::clean(text.lines().next().unwrap_or(text))
}
