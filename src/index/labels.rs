//! Labels, as the map has them: flat, a message may have several, and the
//! same name is the same label on every account, whatever its case. They
//! are kept where the server keeps them. Gmail's labels are Gmail's; a
//! server of keywords keeps each as a keyword, and a message filed is
//! moved from the Inbox to Archive too; a server of folders alone keeps
//! the first in the folder the message is moved to, and letterbox keeps
//! the rest itself. Folders are labels as well, every level of them:
//! `Projects/Grant` is `Projects` and `Grant`.

use super::{FolderState, Index, Kept, Key, Record, fold};
use crate::mail::{self, Folder, Role};
use std::collections::BTreeMap;

/// The longest keyword written, in bytes.
pub const KEYWORD: usize = 50;
/// How long a label given waits on its server before it is let go, in
/// seconds: a server that never says it has it did not take it.
const WAIT: i64 = 24 * 60 * 60;

/// What a server says of a message besides its labels: whether it is in
/// the Inbox, was sent, is a draft, or is marked important.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct System {
    pub inbox: bool,
    pub sent: bool,
    pub draft: bool,
    pub important: bool,
}

/// `s` in IMAP's modified UTF-7 (RFC 3501, 5.1.3): the characters `keep`
/// keeps as they are, `&` as `&-`, and every run of others as `&`, their
/// UTF-16 in base64 with `,` for `/`, and `-`.
pub fn utf7(s: &str, keep: impl Fn(char) -> bool) -> String {
    const B64: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+,";
    let mut out = String::new();
    let mut run: Vec<u16> = Vec::new();
    let flush = |run: &mut Vec<u16>, out: &mut String| {
        if run.is_empty() {
            return;
        }
        out.push('&');
        let bytes: Vec<u8> = run.iter().flat_map(|u| u.to_be_bytes()).collect();
        for chunk in bytes.chunks(3) {
            let n = chunk.len();
            let b = [
                chunk[0],
                *chunk.get(1).unwrap_or(&0),
                *chunk.get(2).unwrap_or(&0),
            ];
            let v = (u32::from(b[0]) << 16) | (u32::from(b[1]) << 8) | u32::from(b[2]);
            for i in 0..=n {
                out.push(B64[((v >> (18 - 6 * i)) & 63) as usize] as char);
            }
        }
        out.push('-');
        run.clear();
    };
    for c in s.chars() {
        if c == '&' {
            flush(&mut run, &mut out);
            out.push_str("&-");
        } else if keep(c) {
            flush(&mut run, &mut out);
            out.push(c);
        } else {
            let mut units = [0u16; 2];
            run.extend_from_slice(c.encode_utf16(&mut units));
        }
    }
    flush(&mut run, &mut out);
    out
}

/// A label as a keyword: letters, digits and `._+/:-` as they are, the
/// rest in modified UTF-7, no longer than `KEYWORD` bytes, cut between
/// one character and the next.
pub fn keyword(label: &str) -> String {
    let keep = |c: char| c.is_ascii_alphanumeric() || "._+/:-".contains(c);
    let mut out = String::new();
    for c in label.trim().chars() {
        let mut more = out.clone();
        more.push(c);
        // Written whole, so that a run of UTF-7 is closed where it ends.
        if utf7(&more, keep).len() > KEYWORD {
            break;
        }
        out = more;
    }
    utf7(&out, keep)
}

/// A keyword as a label: modified UTF-7 decoded.
pub fn unkeyword(keyword: &str) -> String {
    mail::clean(&mail::utf7_imap(keyword))
}

/// A folder's name for a server, as `CREATE` and `RENAME` take it: what
/// is printable in ASCII as it is, the rest in modified UTF-7.
pub fn folder_name(name: &str) -> String {
    utf7(name, |c| (' '..='~').contains(&c))
}

/// The parts of a folder's path, decoded.
fn parts(path: &str, delimiter: Option<&str>) -> Vec<String> {
    let decoded = mail::utf7_imap(path);
    match delimiter {
        Some(d) if !d.is_empty() => decoded.split(d).map(mail::clean).collect(),
        _ => vec![mail::clean(&decoded)],
    }
}

/// The labels a folder is: each level of its path, but for the Inbox it
/// may be under; none for a folder with a role, or one that holds all.
pub fn of_folder(path: &str, state: Option<&FolderState>) -> Vec<String> {
    let role = state.and_then(|s| s.role).unwrap_or(Role::Other);
    if role != Role::Other || state.is_some_and(|s| s.all) {
        return Vec::new();
    }
    let delimiter = state.and_then(|s| s.delimiter.as_deref());
    let mut parts = parts(path, delimiter);
    if parts.len() > 1 && parts[0].eq_ignore_ascii_case("INBOX") {
        parts.remove(0);
    }
    parts.retain(|p| !p.is_empty());
    parts
}

/// Each label as the server has it, once, the first case it is given in
/// kept.
fn add(all: &mut Vec<String>, label: String) {
    if !label.is_empty() && !all.iter().any(|l| fold(l) == fold(&label)) {
        all.push(label);
    }
}

/// The labels the servers keep on `record`: its folders', and Gmail's or
/// its keywords; not those of a system's, as `\Inbox`.
pub fn server(record: &Record, folders: &BTreeMap<(String, String), FolderState>) -> Vec<String> {
    let mut all = Vec::new();
    for p in &record.places {
        let state = folders.get(&(record.key.account.clone(), p.folder.clone()));
        for l in of_folder(&p.folder, state) {
            add(&mut all, l);
        }
    }
    for l in &record.server {
        if l.starts_with('\\') {
            continue;
        }
        let label = unkeyword(l);
        if record.key.id.starts_with("gm:") {
            // Gmail's nested labels are named as folders are, by `/`.
            for part in label.split('/') {
                add(&mut all, mail::clean(part));
            }
        } else {
            add(&mut all, label);
        }
    }
    all
}

/// What the servers say of `record` besides its labels.
pub fn system(record: &Record, folders: &BTreeMap<(String, String), FolderState>) -> System {
    let mut s = System::default();
    for p in &record.places {
        let state = folders.get(&(record.key.account.clone(), p.folder.clone()));
        let role = state.and_then(|s| s.role).or_else(|| {
            p.folder
                .eq_ignore_ascii_case("INBOX")
                .then_some(Role::Inbox)
        });
        match role {
            Some(Role::Inbox) => s.inbox = true,
            Some(Role::Sent) => s.sent = true,
            Some(Role::Drafts) => s.draft = true,
            _ => {}
        }
    }
    for l in &record.server {
        match l.to_ascii_lowercase().as_str() {
            "\\inbox" => s.inbox = true,
            "\\sent" => s.sent = true,
            "\\draft" => s.draft = true,
            "\\important" => s.important = true,
            _ => {}
        }
    }
    s
}

impl Index {
    /// The labels of the message `key`, as shown: the servers', those
    /// letterbox keeps, and those given and not yet confirmed; but those
    /// taken off and not yet confirmed.
    pub fn labels(&self, key: &Key) -> Vec<String> {
        let Some(record) = self.records.get(key) else {
            return Vec::new();
        };
        let mut all = server(record, &self.folders);
        for l in self.keep.labels.get(key).into_iter().flatten() {
            add(&mut all, l.clone());
        }
        for ((k, folded), w) in self.keep.pending.range((key.clone(), String::new())..) {
            if k != key {
                break;
            }
            if w.on {
                add(&mut all, w.label.clone());
            } else {
                all.retain(|l| fold(l) != *folded);
            }
        }
        all
    }

    /// What the servers say of the message `key` besides its labels; in
    /// the Inbox no longer once it is filed, before the server says so.
    pub fn system(&self, key: &Key) -> System {
        let Some(record) = self.records.get(key) else {
            return System::default();
        };
        let mut s = system(record, &self.folders);
        let filed = self
            .keep
            .pending
            .range((key.clone(), String::new())..)
            .take_while(|((k, _), _)| k == key)
            .any(|(_, w)| w.on);
        if filed {
            s.inbox = false;
        }
        s
    }

    /// What waits on the servers of `account` and they now have, or has
    /// waited too long: no longer pending.
    pub fn settled(&self, account: &str, now: i64) -> Vec<Kept> {
        self.keep
            .pending
            .iter()
            .filter(|((k, _), _)| k.account == account)
            .filter(|((k, folded), w)| {
                let has = self
                    .records
                    .get(k)
                    .is_some_and(|r| server(r, &self.folders).iter().any(|l| fold(l) == *folded));
                has == w.on || now - w.at > WAIT || !self.records.contains_key(k)
            })
            .map(|((k, _), w)| Kept::Settled {
                key: k.clone(),
                label: w.label.clone(),
            })
            .collect()
    }
}

/// The folder a label is, of `folders`: the one whose own name is it,
/// the shortest path of those that are.
pub fn folder_for<'a>(folders: &'a [Folder], label: &str) -> Option<&'a Folder> {
    folders
        .iter()
        .filter(|f| f.selectable && f.role == Role::Other && !f.all)
        .filter(|f| fold(&f.name) == fold(label))
        .min_by_key(|f| (f.path.len(), f.path.clone()))
}
