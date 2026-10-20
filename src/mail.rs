//! Mail as letterbox shows it: folders, in a mail app's order; each
//! message's summary, as listed; and a message read whole. Messages are
//! parsed from their RFC 5322 bytes, whatever store they came from.

use mail_parser::{Address, MessageParser, MimeHeaders, PartType};
use serde::Serialize;

const PREVIEW_CHARS: usize = 240;

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum Role {
    Inbox,
    Drafts,
    Sent,
    Archive,
    Junk,
    Trash,
    Other,
}

impl Role {
    /// The role a folder plays: by its special-use attribute, as RFC 6154
    /// has servers mark them, else by the names servers give them.
    pub fn of(path: &str, name: &str, special: Option<Role>) -> Role {
        if path.eq_ignore_ascii_case("INBOX") {
            return Role::Inbox;
        }
        if let Some(r) = special {
            return r;
        }
        match name.to_lowercase().as_str() {
            "drafts" | "draft" => Role::Drafts,
            "sent" | "sent items" | "sent messages" | "sent mail" => Role::Sent,
            "archive" | "archives" => Role::Archive,
            "junk" | "junk email" | "junk e-mail" | "spam" | "bulk mail" => Role::Junk,
            "trash" | "deleted items" | "deleted messages" | "bin" => Role::Trash,
            _ => Role::Other,
        }
    }

    pub fn icon(self) -> &'static str {
        match self {
            Role::Inbox => "✉",
            Role::Drafts => "✎",
            Role::Sent => "➤",
            Role::Archive => "▤",
            Role::Junk => "⊘",
            Role::Trash => "✕",
            Role::Other => "▸",
        }
    }
}

#[derive(Clone, Debug, Serialize)]
pub struct Folder {
    /// Its name on the server, as commands give it.
    pub path: String,
    /// Its own name, decoded, without its parents'.
    pub name: String,
    #[serde(skip)]
    pub delimiter: Option<String>,
    /// How far under another folder it is shown.
    pub depth: usize,
    pub role: Role,
    pub selectable: bool,
    pub total: Option<u32>,
    pub unseen: Option<u32>,
}

impl Folder {
    pub fn new(
        path: &str,
        delimiter: Option<&str>,
        special: Option<Role>,
        selectable: bool,
    ) -> Self {
        let parts = parts(path, delimiter);
        let leaf = parts.last().cloned().unwrap_or_default();
        let role = Role::of(path, &leaf, special);
        Folder {
            path: path.to_string(),
            name: if role == Role::Inbox {
                "Inbox".to_string()
            } else {
                leaf
            },
            delimiter: delimiter.map(str::to_string),
            // Folders with a role are shown at the top, wherever they are.
            depth: if role == Role::Other {
                parts.len().saturating_sub(1)
            } else {
                0
            },
            role,
            selectable,
            total: None,
            unseen: None,
        }
    }
}

/// A folder's path, decoded, as the names it is under and its own.
fn parts(path: &str, delimiter: Option<&str>) -> Vec<String> {
    let decoded = utf7_imap(path);
    match delimiter {
        Some(d) if !d.is_empty() => decoded.split(d).map(str::to_string).collect(),
        _ => vec![decoded],
    }
}

/// Put `folders` in a mail app's order: Inbox, Drafts, Sent, Archive, Junk
/// and Trash first, then the rest by name, each under its parent.
pub fn arrange(folders: &mut [Folder]) {
    let roles: std::collections::HashMap<String, Role> =
        folders.iter().map(|f| (f.path.clone(), f.role)).collect();
    let key = |f: &Folder| -> Vec<(Role, String)> {
        if f.role != Role::Other {
            return vec![(f.role, f.path.to_lowercase())];
        }
        let raw: Vec<&str> = match f.delimiter.as_deref() {
            Some(d) if !d.is_empty() => f.path.split(d).collect(),
            _ => vec![f.path.as_str()],
        };
        let names = parts(&f.path, f.delimiter.as_deref());
        (0..raw.len())
            .map(|i| {
                let prefix = raw[..=i].join(f.delimiter.as_deref().unwrap_or(""));
                let role = roles.get(&prefix).copied().unwrap_or(Role::Other);
                // A folder under Inbox stays under it; under Sent, say, it
                // is with the rest, its parent having moved up.
                let role = if role == Role::Inbox {
                    role
                } else {
                    Role::Other
                };
                (
                    role,
                    names.get(i).cloned().unwrap_or_default().to_lowercase(),
                )
            })
            .collect()
    };
    folders.sort_by_cached_key(key);
}

/// A name in IMAP's modified UTF-7 (RFC 3501, 5.1.3), as `Entw&APw-rfe`
/// for `Entwürfe`, decoded; what is not valid is kept as it is.
pub fn utf7_imap(name: &str) -> String {
    let mut out = String::new();
    let mut rest = name;
    while let Some(at) = rest.find('&') {
        out.push_str(&rest[..at]);
        let after = &rest[at + 1..];
        let Some(end) = after.find('-') else {
            out.push_str(&rest[at..]);
            return out;
        };
        let encoded = &after[..end];
        if encoded.is_empty() {
            out.push('&');
        } else {
            match decode_utf16_base64(encoded) {
                Some(s) => out.push_str(&s),
                None => out.push_str(&rest[at..at + 2 + end]),
            }
        }
        rest = &after[end + 1..];
    }
    out.push_str(rest);
    out
}

fn decode_utf16_base64(s: &str) -> Option<String> {
    let mut bits: u32 = 0;
    let mut n = 0;
    let mut bytes = Vec::new();
    for c in s.chars() {
        let v = match c {
            'A'..='Z' => c as u32 - 'A' as u32,
            'a'..='z' => c as u32 - 'a' as u32 + 26,
            '0'..='9' => c as u32 - '0' as u32 + 52,
            '+' => 62,
            ',' => 63,
            _ => return None,
        };
        bits = (bits << 6) | v;
        n += 6;
        if n >= 8 {
            n -= 8;
            bytes.push((bits >> n) as u8);
            bits &= (1 << n) - 1;
        }
    }
    let units: Vec<u16> = bytes
        .as_chunks::<2>()
        .0
        .iter()
        .map(|p| u16::from_be_bytes([p[0], p[1]]))
        .collect();
    String::from_utf16(&units).ok()
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize)]
pub struct Flags {
    pub seen: bool,
    pub flagged: bool,
    pub answered: bool,
}

/// A message as it is listed.
#[derive(Clone, Debug, Serialize)]
pub struct Summary {
    pub uid: u32,
    /// The sender's name, else their address.
    pub from: String,
    pub address: String,
    pub to: String,
    pub subject: String,
    /// Seconds since 1970, as the message is dated.
    pub date: Option<i64>,
    #[serde(flatten)]
    pub flags: Flags,
    /// The first words of its text.
    pub preview: String,
    pub size: u32,
}

/// The summary of a message from its header, and as much of its text after
/// the header as there is, as the first `PREVIEW_BYTES` of it.
pub fn summary(uid: u32, flags: Flags, size: u32, raw: &[u8]) -> Summary {
    let parsed = MessageParser::default().parse(raw);
    let Some(m) = parsed else {
        return Summary {
            uid,
            from: String::new(),
            address: String::new(),
            to: String::new(),
            subject: String::new(),
            date: None,
            flags,
            preview: String::new(),
            size,
        };
    };
    let first = m.from().and_then(Address::first);
    let address = first
        .and_then(|a| a.address())
        .map(clean)
        .unwrap_or_default();
    let from = first
        .and_then(|a| a.name())
        .map(clean)
        .filter(|n| !n.is_empty())
        .unwrap_or_else(|| address.clone());
    let preview: String = m.body_text(0).map(|t| preview(&t)).unwrap_or_default();
    Summary {
        uid,
        from,
        address,
        to: m.to().map(addresses).unwrap_or_default().join(", "),
        subject: m.subject().map(clean).unwrap_or_default(),
        date: m.date().map(|d| d.to_timestamp()),
        flags,
        preview,
        size,
    }
}

/// The text's first words, on one line, without what it quotes.
fn preview(text: &str) -> String {
    let words = text
        .lines()
        .filter(|l| !l.trim_start().starts_with('>'))
        .flat_map(str::split_whitespace);
    let mut out = String::new();
    for w in words {
        if out.chars().count() + w.chars().count() > PREVIEW_CHARS {
            break;
        }
        if !out.is_empty() {
            out.push(' ');
        }
        out.push_str(w);
    }
    clean(&out)
}

#[derive(Clone, Debug, Serialize)]
pub struct Attachment {
    pub name: String,
    pub mime: String,
    pub size: usize,
}

/// A message read whole.
#[derive(Clone, Debug, Serialize)]
pub struct Message {
    pub uid: u32,
    pub subject: String,
    pub from: String,
    pub to: Vec<String>,
    pub cc: Vec<String>,
    pub reply_to: Vec<String>,
    pub date: Option<i64>,
    pub message_id: Option<String>,
    /// Its text: the plain part, else what its HTML says.
    pub text: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub html: Option<String>,
    pub attachments: Vec<Attachment>,
    #[serde(skip)]
    pub raw: Vec<u8>,
}

pub fn message(uid: u32, raw: Vec<u8>) -> anyhow::Result<Message> {
    let m = MessageParser::default()
        .parse(&raw)
        .ok_or_else(|| anyhow::anyhow!("message {uid} cannot be read"))?;
    let list = |a: Option<&Address>| a.map(addresses).unwrap_or_default();
    let html = m
        .parts
        .iter()
        .any(|p| matches!(p.body, PartType::Html(_)))
        .then(|| m.body_html(0).map(|h| h.into_owned()))
        .flatten();
    let attachments = m
        .attachments()
        .map(|p| Attachment {
            name: p
                .attachment_name()
                .map(clean)
                .unwrap_or_else(|| "unnamed".into()),
            mime: p
                .content_type()
                .map(|c| match c.subtype() {
                    Some(s) => format!("{}/{s}", c.ctype()),
                    None => c.ctype().to_string(),
                })
                .unwrap_or_default(),
            size: p.contents().len(),
        })
        .collect();
    let text = m.body_text(0).map(|t| body(&t)).unwrap_or_default();
    let message = Message {
        uid,
        subject: m.subject().map(clean).unwrap_or_default(),
        from: list(m.from()).join(", "),
        to: list(m.to()),
        cc: list(m.cc()),
        reply_to: list(m.reply_to()),
        date: m.date().map(|d| d.to_timestamp()),
        message_id: m.message_id().map(clean),
        text,
        html,
        attachments,
        raw: Vec::new(),
    };
    Ok(Message { raw, ..message })
}

/// The attachments of a message, by name, and what they hold.
pub fn attachments(raw: &[u8]) -> Vec<(String, Vec<u8>)> {
    let Some(m) = MessageParser::default().parse(raw) else {
        return Vec::new();
    };
    m.attachments()
        .map(|p| {
            let name = p
                .attachment_name()
                .map(clean)
                .unwrap_or_else(|| "unnamed".into());
            (name, p.contents().to_vec())
        })
        .collect()
}

/// `Name <address>`, or either alone, for each in `a`.
fn addresses(a: &Address) -> Vec<String> {
    a.iter()
        .map(|x| match (x.name().map(clean), x.address().map(clean)) {
            (Some(n), Some(a)) if !n.is_empty() && n != a => format!("{n} <{a}>"),
            (_, Some(a)) => a,
            (Some(n), None) => n,
            (None, None) => String::new(),
        })
        .filter(|s| !s.is_empty())
        .collect()
}

/// A message's text, its lines ended alike, without the blank lines it ends
/// with or more than one blank line together, and with nothing a terminal
/// would take as a command.
fn body(text: &str) -> String {
    let mut out = String::new();
    let mut blanks = 0;
    for line in text.lines() {
        let line = controls(&line.replace('\t', "    "));
        let line = line.trim_end();
        if line.is_empty() {
            blanks += 1;
            continue;
        }
        if !out.is_empty() {
            out.push_str(if blanks > 0 { "\n\n" } else { "\n" });
        }
        blanks = 0;
        out.push_str(line);
    }
    out
}

/// `s` without control characters: mail is from strangers, and an escape
/// in it would otherwise reach the terminal.
pub fn clean(s: &str) -> String {
    controls(s).trim().to_string()
}

fn controls(s: &str) -> String {
    s.chars()
        .map(|c| if c.is_control() { ' ' } else { c })
        .collect()
}
