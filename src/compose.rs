//! A message being written: the buffer's headers, a reply filled in from
//! the message answered, and the bytes sent, as RFC 5322.

use crate::mail::{self, Message};
use crate::ui;
use base64::Engine;
use jiff::Zoned;

/// A header line above the blank line that ends them.
#[derive(Clone, Copy)]
enum Head {
    To,
    Cc,
    Subject,
}

/// What was written: who it is to, and the text below the blank line.
pub struct Draft {
    pub to: Vec<String>,
    pub cc: Vec<String>,
    pub subject: String,
    pub body: String,
}

/// Read `To`, `Cc` and `Subject` from the lines above the first blank line
/// of `text`. Anything else there, or a missing `To`, is said and nothing
/// is sent.
pub fn parse(text: &str) -> Result<Draft, String> {
    let text = text.strip_suffix('\n').unwrap_or(text);
    let Some((head, body)) = text.split_once("\n\n") else {
        return Err("a blank line ends the headers".to_string());
    };
    let mut to = String::new();
    let mut cc = String::new();
    let mut subject = String::new();
    let mut which = None;
    for line in head.lines() {
        if line.starts_with([' ', '\t']) {
            let Some(slot) = which else {
                return Err("a header is a name, a colon, and its value".to_string());
            };
            let slot = match slot {
                Head::To => &mut to,
                Head::Cc => &mut cc,
                Head::Subject => &mut subject,
            };
            slot.push(' ');
            slot.push_str(line.trim());
            continue;
        }
        let Some((name, value)) = line.split_once(':') else {
            return Err("a header is a name, a colon, and its value".to_string());
        };
        let head = match name.trim().to_ascii_lowercase().as_str() {
            "to" => Head::To,
            "cc" => Head::Cc,
            "subject" => Head::Subject,
            _ => return Err("only To, Cc and Subject go above the message".to_string()),
        };
        which = Some(head);
        let value = value.trim();
        match head {
            Head::To => to = value.to_string(),
            Head::Cc => cc = value.to_string(),
            Head::Subject => subject = value.to_string(),
        }
    }
    let to = addresses(&to)?;
    if to.is_empty() {
        return Err("say who it is to".to_string());
    }
    let cc = addresses(&cc)?;
    Ok(Draft {
        to,
        cc,
        subject,
        body: body.to_string(),
    })
}

/// `Re: …` unless `subject` already begins so. The check is by characters:
/// a subject can start with a character of more than one byte.
pub fn reply_subject(subject: &str) -> String {
    let subject = subject.trim();
    let mut chars = subject.chars();
    let re = matches!(
        (chars.next(), chars.next(), chars.next()),
        (Some('r' | 'R'), Some('e' | 'E'), Some(':'))
    );
    if re {
        subject.to_string()
    } else if subject.is_empty() {
        "Re:".to_string()
    } else {
        format!("Re: {subject}")
    }
}

/// The buffer a reply opens with, and the row its point starts on: the
/// first line of the body, above what it quotes.
pub fn reply(message: &Message) -> (String, usize) {
    let to = message
        .reply_to
        .first()
        .cloned()
        .filter(|s| !s.is_empty())
        .unwrap_or_else(|| message.from.clone());
    let subject = reply_subject(&message.subject);
    let when = message.date.map(ui::long_date).filter(|s| !s.is_empty());
    let wrote = match &when {
        Some(w) => format!("On {w}, {} wrote:", message.from),
        None => format!("{} wrote:", message.from),
    };
    let quoted = quote(&message.text);
    let text = format!("To: {to}\nCc: \nSubject: {subject}\n\n\n{wrote}\n{quoted}");
    let row = 4;
    (text, row)
}

/// An empty message; point is at the end of `To:`, to type the address.
/// The blank line after `Subject` ends the headers, and the line under it
/// is where the body is written.
pub fn blank() -> (String, usize, usize) {
    ("To: \nCc: \nSubject: \n\n\n".to_string(), 0, 4)
}

/// `In-Reply-To` and `References` for a reply to `message`: its id, and the
/// references it already carried, then that id.
pub fn thread(message: &Message) -> (Option<String>, Option<String>) {
    let id = message
        .message_id
        .clone()
        .filter(|s| !s.is_empty())
        .or_else(|| mail::field(&message.raw, "Message-ID"));
    let id = id.map(|s| mail::brackets(&s));
    let mut refs: Vec<String> = mail::field(&message.raw, "References")
        .map(|s| {
            s.split_whitespace()
                .map(mail::brackets)
                .filter(|s| s.contains('@'))
                .collect()
        })
        .unwrap_or_default();
    if let Some(id) = &id
        && !refs.iter().any(|r| r == id)
    {
        refs.push(id.clone());
    }
    let references = (!refs.is_empty()).then(|| refs.join(" "));
    (id, references)
}

/// `draft` as the bytes sent from `from`, answering `in_reply_to` when it
/// is a reply. The body is UTF-8, declared `8bit`; a server without
/// `8BITMIME` has it rewritten before it is sent.
pub fn bytes(
    from: &str,
    draft: &Draft,
    in_reply_to: Option<&str>,
    references: Option<&str>,
) -> Vec<u8> {
    let date = Zoned::now().strftime("%a, %d %b %Y %H:%M:%S %z");
    let id = message_id(from);
    let mut head = format!("From: {from}\r\nTo: {}\r\n", draft.to.join(", "));
    if !draft.cc.is_empty() {
        head.push_str(&format!("Cc: {}\r\n", draft.cc.join(", ")));
    }
    head.push_str(&format!("Subject: {}\r\n", encoded(&draft.subject)));
    head.push_str(&format!("Date: {date}\r\n"));
    head.push_str(&format!("Message-ID: {id}\r\n"));
    if let Some(id) = in_reply_to.filter(|s| !s.is_empty()) {
        head.push_str(&format!("In-Reply-To: {}\r\n", mail::brackets(id)));
    }
    if let Some(refs) = references.filter(|s| !s.is_empty()) {
        head.push_str(&format!("References: {refs}\r\n"));
    }
    head.push_str("MIME-Version: 1.0\r\n");
    head.push_str("Content-Type: text/plain; charset=utf-8\r\n");
    head.push_str("Content-Transfer-Encoding: 8bit\r\n");
    let body = draft.body.replace("\r\n", "\n").replace('\r', "\n");
    let body = body.replace('\n', "\r\n");
    let body = if body.ends_with("\r\n") {
        body
    } else {
        format!("{body}\r\n")
    };
    format!("{head}\r\n{body}").into_bytes()
}

fn quote(text: &str) -> String {
    if text.is_empty() {
        return ">\n".to_string();
    }
    let mut out = String::new();
    for line in text.lines() {
        out.push_str("> ");
        out.push_str(line);
        out.push('\n');
    }
    out
}

fn addresses(s: &str) -> Result<Vec<String>, String> {
    if s.trim().is_empty() {
        return Ok(Vec::new());
    }
    let mut out = Vec::new();
    for part in split_addrs(s) {
        if addr_spec(&part).is_none() {
            return Err(format!("{part} is not an address"));
        }
        out.push(part);
    }
    Ok(out)
}

/// The addresses in a header value, split on commas that are not inside a
/// name's `<...>` or a quoted name.
pub(crate) fn split_addrs(s: &str) -> Vec<String> {
    let mut out = Vec::new();
    let mut cur = String::new();
    let mut angle = false;
    let mut quote = false;
    for c in s.chars() {
        match c {
            '"' if !angle => quote = !quote,
            '<' if !quote => angle = true,
            '>' if !quote => angle = false,
            ',' if !quote && !angle => {
                if !cur.trim().is_empty() {
                    out.push(cur.trim().to_string());
                }
                cur.clear();
                continue;
            }
            _ => {}
        }
        cur.push(c);
    }
    if !cur.trim().is_empty() {
        out.push(cur.trim().to_string());
    }
    out
}

/// The address itself, out of `Name <address>` or a bare address.
pub fn addr_spec(s: &str) -> Option<String> {
    let s = s.trim();
    let spec = if let Some(start) = s.rfind('<') {
        s[start + 1..].split('>').next()?.trim()
    } else {
        s
    };
    let spec = spec.trim();
    (spec.contains('@') && !spec.contains(char::is_whitespace)).then(|| spec.to_string())
}

/// `s` as a header value: as it is when it is ASCII, else as encoded words,
/// and with no line break a header could not hold. Each word stays under
/// the 75 characters a header line allows, and ends on a character, so a
/// multibyte character is not split between two words.
fn encoded(s: &str) -> String {
    let s = s.replace(['\r', '\n'], " ");
    if s.is_ascii() {
        return s;
    }
    let mut out = String::new();
    let mut rest = s.as_str();
    while !rest.is_empty() {
        let mut end = 0;
        for (i, c) in rest.char_indices() {
            if i + c.len_utf8() > 45 {
                break;
            }
            end = i + c.len_utf8();
        }
        if end == 0 {
            end = rest.chars().next().map(char::len_utf8).unwrap_or(0);
        }
        let (chunk, next) = rest.split_at(end);
        rest = next;
        if !out.is_empty() {
            out.push(' ');
        }
        let b = base64::engine::general_purpose::STANDARD.encode(chunk.as_bytes());
        out.push_str(&format!("=?utf-8?b?{b}?="));
    }
    out
}

fn message_id(from: &str) -> String {
    let domain = from.rsplit_once('@').map(|(_, d)| d).unwrap_or("letterbox");
    let ticks = jiff::Timestamp::now().as_millisecond();
    let mut rand = [0u8; 8];
    let hex = match getrandom::fill(&mut rand) {
        Ok(()) => rand.iter().map(|b| format!("{b:02x}")).collect::<String>(),
        Err(_) => "0".to_string(),
    };
    format!("<{ticks}.{hex}@{domain}>")
}
