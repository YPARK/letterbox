//! Mail as letterbox shows it: folders, in a mail app's order; each
//! message's summary, as listed; and a message read whole. Messages are
//! parsed from their RFC 5322 bytes, whatever store they came from.

use mail_parser::decoders::html::html_to_text;
use mail_parser::{Address, MessageParser, MimeHeaders, PartType};
use serde::{Deserialize, Serialize};

const PREVIEW_CHARS: usize = 240;

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
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

    /// Its name, as the folders of it of every account are named together.
    pub fn name(self) -> &'static str {
        match self {
            Role::Inbox => "Inbox",
            Role::Drafts => "Drafts",
            Role::Sent => "Sent",
            Role::Archive => "Archive",
            Role::Junk => "Junk",
            Role::Trash => "Trash",
            Role::Other => "Other",
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
    /// Whether it holds every message of the account, as Gmail's All Mail.
    #[serde(skip_serializing_if = "std::ops::Not::not")]
    pub all: bool,
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
            all: false,
            total: None,
            unseen: None,
        }
    }
}

/// A folder's path, decoded, as the names it is under and its own; with
/// nothing a terminal would take as a command, as a server could send.
fn parts(path: &str, delimiter: Option<&str>) -> Vec<String> {
    let decoded = utf7_imap(path);
    match delimiter {
        Some(d) if !d.is_empty() => decoded.split(d).map(controls).collect(),
        _ => vec![controls(&decoded)],
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

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Flags {
    pub seen: bool,
    pub flagged: bool,
    pub answered: bool,
}

/// A message as it is listed.
#[derive(Clone, Debug, Default, Serialize)]
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
    /// Its `Message-ID`, as the header has it, with the brackets; empty
    /// when it has none.
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub message_id: String,
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
            message_id: String::new(),
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
    let preview: String = m
        .body_text(0)
        .map(|t| preview(&t, PREVIEW_CHARS))
        .unwrap_or_default();
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
        message_id: m
            .message_id()
            .map(clean)
            .filter(|s| !s.is_empty())
            .map(|s| brackets(&s))
            .unwrap_or_default(),
    }
}

/// The text's first words, no more than `chars` of them, on one line, without
/// what it quotes.
pub fn preview(text: &str, chars: usize) -> String {
    let words = text
        .lines()
        .filter(|l| !l.trim_start().starts_with('>'))
        .flat_map(str::split_whitespace);
    let mut out = String::new();
    for w in words {
        if out.chars().count() + w.chars().count() > chars {
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
    /// The web addresses it links to, in its text or its HTML, in order,
    /// each once.
    pub links: Vec<String>,
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
    let attachments = m.attachments().map(attachment).collect();
    let text = plain(&m).map(|t| body(&t)).unwrap_or_default();
    let links = links(&text, html.as_deref());
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
        links,
        raw: Vec::new(),
    };
    Ok(Message { raw, ..message })
}

/// The most links a message is taken to have.
const LINKS: usize = 200;

/// The web addresses in `text`, and in `html`'s links, in order, each once:
/// without the punctuation that ends a sentence after one, or the brackets
/// around it.
pub fn links(text: &str, html: Option<&str>) -> Vec<String> {
    let mut found: Vec<String> = Vec::new();
    let mut add = |url: &str| {
        if let Some(url) = web_address(url)
            && !found.contains(&url)
            && found.len() < LINKS
        {
            found.push(url);
        }
    };
    let words = |s: &str| -> Vec<String> {
        let mut out = Vec::new();
        let mut rest = s;
        while let Some(at) = rest.find("http") {
            let from = &rest[at..];
            if from.starts_with("https://") || from.starts_with("http://") {
                let end = from
                    .find(|c: char| c.is_whitespace() || "<>\"'`".contains(c))
                    .unwrap_or(from.len());
                out.push(from[..end].to_string());
                rest = &from[end..];
            } else {
                rest = &from[4..];
            }
        }
        out
    };
    for w in words(text) {
        add(&w);
    }
    if let Some(h) = html {
        let mut rest = h;
        while let Some(at) = rest.find("href=") {
            let from = &rest[at + 5..];
            let quote = from.chars().next().filter(|c| *c == '"' || *c == '\'');
            let value = match quote {
                Some(q) => from[1..].split(q).next().unwrap_or_default(),
                None => from
                    .split(|c: char| c.is_whitespace() || c == '>')
                    .next()
                    .unwrap_or_default(),
            };
            add(value.trim());
            rest = from;
        }
    }
    found
}

/// `url` as `links` keeps it, if it is a web address: without the
/// punctuation that ends a sentence after one, or the brackets around it.
fn web_address(url: &str) -> Option<String> {
    let url = url
        .trim_end_matches(['.', ',', ';', ':', '!', '?', ')', ']', '>', '\'', '"'])
        .replace("&amp;", "&");
    let web = url.starts_with("https://") || url.starts_with("http://");
    (web && url.len() > 10).then(|| clean(&url))
}

/// What each of `links` is called: the text of the first link to it in
/// `html` that has any; else, behind a redirect, where it goes; else none.
pub fn link_names(links: &[String], html: Option<&str>) -> Vec<Option<String>> {
    let anchors = html.map(crate::papers::anchors).unwrap_or_default();
    links
        .iter()
        .map(|url| {
            anchors
                .iter()
                .filter(|(href, _, _)| web_address(href.trim()).as_ref() == Some(url))
                .map(|(_, inner, _)| {
                    let text = crate::papers::text_of(inner);
                    clean(&text.split_whitespace().collect::<Vec<_>>().join(" "))
                })
                .find(|text| !text.is_empty() && text != url)
                .or_else(|| Some(crate::papers::unwrapped(url)).filter(|to| to != url))
        })
        .collect()
}

/// The attachments of a message, by name, and what they hold.
pub fn attachments(raw: &[u8]) -> Vec<(Attachment, Vec<u8>)> {
    let Some(m) = MessageParser::default().parse(raw) else {
        return Vec::new();
    };
    m.attachments()
        .map(|p| (attachment(p), p.contents().to_vec()))
        .collect()
}

/// The images a message's HTML shows from its own parts, by `cid:`: each
/// part's Content-ID, its type, and what it holds.
pub fn inline_images(raw: &[u8]) -> Vec<(String, String, Vec<u8>)> {
    let Some(m) = MessageParser::default().parse(raw) else {
        return Vec::new();
    };
    m.parts
        .iter()
        .filter_map(|p| {
            let id = p.content_id()?.trim_matches(['<', '>']).to_string();
            let c = p.content_type()?;
            let mime = format!("{}/{}", c.ctype(), c.subtype()?).to_lowercase();
            mime.starts_with("image/")
                .then(|| (id, mime, p.contents().to_vec()))
        })
        .collect()
}

/// An attachment as it is listed: its name, its type, its size. A part
/// that says no type is taken as bytes.
fn attachment(p: &mail_parser::MessagePart) -> Attachment {
    Attachment {
        name: p
            .attachment_name()
            .map(clean)
            .unwrap_or_else(|| "unnamed".into()),
        mime: p
            .content_type()
            .map(|c| match c.subtype() {
                Some(s) => clean(&format!("{}/{s}", c.ctype())),
                None => clean(c.ctype()),
            })
            .filter(|m| !m.is_empty())
            .unwrap_or_else(|| "application/octet-stream".into()),
        size: p.contents().len(),
    }
}

/// `Name <address>`, or either alone, for each in `a`.
fn addresses(a: &Address) -> Vec<String> {
    a.iter()
        .map(|x| match (x.name().map(clean), x.address().map(clean)) {
            (Some(n), Some(a)) if !n.is_empty() && n != a => format!("{} <{a}>", phrase(&n)),
            (_, Some(a)) => a,
            (Some(n), None) => n,
            (None, None) => String::new(),
        })
        .filter(|s| !s.is_empty())
        .collect()
}

/// A name as written before its address: quoted, as it came, when it holds
/// what would part it from its address or from the next, as `Yan, Joseph`.
fn phrase(name: &str) -> String {
    if name.contains([',', ';', ':', '<', '>', '@', '"', '\\', '(', ')', '[', ']']) {
        format!("\"{}\"", name.replace('\\', "\\\\").replace('"', "\\\""))
    } else {
        name.to_string()
    }
}

/// A message's text: its plain text, else its HTML read in lines, a line
/// for each heading, block and row.
fn plain<'x>(m: &'x mail_parser::Message<'x>) -> Option<std::borrow::Cow<'x, str>> {
    match m.text_part(0).map(|p| &p.body) {
        Some(PartType::Html(h)) => Some(html_to_text(&blocks(h)).into()),
        _ => m.body_text(0),
    }
}

/// The tags of HTML that begin and end a line of it read as text:
/// mail-parser ends one only at `<br>` and `</p>`.
const BLOCKS: &[&str] = &[
    "div",
    "h1",
    "h2",
    "h3",
    "h4",
    "h5",
    "h6",
    "li",
    "tr",
    "table",
    "ul",
    "ol",
    "blockquote",
    "pre",
    "hr",
    "section",
    "article",
    "header",
    "footer",
];

/// What mail-parser leaves out of the text, nothing in it read.
const UNREAD: &[&str] = &["head", "script", "style", "template"];

/// `html` with a `<br>` at each tag of a block, begun or ended, but where
/// a line has just ended, and none in what is not read.
fn blocks(html: &str) -> String {
    let mut out = String::with_capacity(html.len());
    let mut rest = html;
    let mut unread: Option<&str> = None;
    // A line just ended, or nothing written yet: no line more.
    let mut ended = true;
    while let Some(at) = rest.find('<') {
        let Some(end) = tag_end(&rest[at..]).map(|e| at + e) else {
            break;
        };
        let text = &rest[..at];
        ended &= text.trim().is_empty();
        out.push_str(text);
        let tag = &rest[at + 1..end - 1];
        let closing = tag.starts_with('/');
        let name = tag
            .trim_start_matches('/')
            .split(|c: char| c.is_whitespace() || c == '/')
            .next()
            .unwrap_or("");
        match unread {
            Some(u) => {
                if closing && name.eq_ignore_ascii_case(u) {
                    unread = None;
                }
            }
            None => {
                if !closing && let Some(u) = UNREAD.iter().find(|u| u.eq_ignore_ascii_case(name)) {
                    unread = Some(u);
                } else if BLOCKS.iter().any(|b| b.eq_ignore_ascii_case(name)) && !ended {
                    out.push_str("<br>");
                    ended = true;
                } else if name.eq_ignore_ascii_case("br")
                    || closing && name.eq_ignore_ascii_case("p")
                {
                    ended = true;
                }
            }
        }
        out.push_str(&rest[at..end]);
        rest = &rest[end..];
    }
    out.push_str(rest);
    out
}

/// Where the tag `s` begins with ends, past its `>`: a comment at its
/// `-->`, else at the first `>` outside an attribute's quotes.
fn tag_end(s: &str) -> Option<usize> {
    if s.starts_with("<!--") {
        return s.find("-->").map(|e| e + 3);
    }
    let (mut quote, mut last) = (None, ' ');
    for (i, c) in s.char_indices() {
        match (quote, c) {
            (Some(q), c) if c == q => quote = None,
            (None, '"' | '\'') if last == '=' => quote = Some(c),
            (None, '>') => return Some(i + 1),
            _ => {}
        }
        if !c.is_whitespace() {
            last = c;
        }
    }
    None
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

/// `s` without control characters but its line breaks: what a server says
/// as why something failed, shown a line at a time.
pub fn clean_lines(s: &str) -> String {
    s.lines().map(controls).collect::<Vec<_>>().join("\n")
}

pub fn controls(s: &str) -> String {
    s.chars()
        .map(|c| if c.is_control() { ' ' } else { c })
        .collect()
}

/// The value of the header `name` in `raw`, unfolded onto one line; none
/// when the message has no such header. The header block ends at the first
/// blank line.
pub fn field(raw: &[u8], name: &str) -> Option<String> {
    let text = header_block(raw);
    let mut value: Option<String> = None;
    let mut matches = false;
    for line in text.lines() {
        if line.starts_with([' ', '\t']) {
            if matches {
                let v = value.get_or_insert_with(String::new);
                if !v.is_empty() {
                    v.push(' ');
                }
                v.push_str(line.trim());
            }
            continue;
        }
        let Some((n, v)) = line.split_once(':') else {
            matches = false;
            continue;
        };
        matches = n.trim().eq_ignore_ascii_case(name);
        if matches {
            value = Some(v.trim().to_string());
        }
    }
    value.filter(|v| !v.is_empty())
}

/// `id` with the angle brackets a `Message-ID` is written in, unless it
/// already has them.
pub(crate) fn brackets(id: &str) -> String {
    let id = id.trim();
    if id.starts_with('<') {
        id.to_string()
    } else {
        format!("<{id}>")
    }
}

fn header_block(raw: &[u8]) -> String {
    String::from_utf8_lossy(&raw[..header_end(raw)]).into_owned()
}

/// Where the headers end: the earlier of `\r\n\r\n` and `\n\n`, or the
/// whole message when it has no blank line. Only that prefix is decoded.
fn header_end(raw: &[u8]) -> usize {
    let mut i = 0;
    while i + 1 < raw.len() {
        if raw[i] == b'\n' && raw[i + 1] == b'\n' {
            return i;
        }
        if i + 3 < raw.len()
            && raw[i] == b'\r'
            && raw[i + 1] == b'\n'
            && raw[i + 2] == b'\r'
            && raw[i + 3] == b'\n'
        {
            return i;
        }
        i += 1;
    }
    raw.len()
}
