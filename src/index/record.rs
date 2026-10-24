//! What the index keeps of a message: what it is known by, where it is
//! on its server, and what the map reads of it. A message is known by its
//! account and Gmail's ID for it, else its `Message-ID`, else a hash of its
//! date, sender, subject and size; never by a UID, which a move changes.

use crate::mail::{self, Flags, Role};
use mail_parser::{Address, MessageParser};
use serde::{Deserialize, Serialize};

/// How many bytes of a message's text are kept, to read it by.
pub const PREVIEW: usize = 512;

/// FNV-1a, 64 bits: a hash that is the same on every machine and every
/// run, as the standard library's is not.
pub fn fnv(bytes: &[u8]) -> u64 {
    let mut h: u64 = 0xcbf2_9ce4_8422_2325;
    for &b in bytes {
        h ^= u64::from(b);
        h = h.wrapping_mul(0x0100_0000_01b3);
    }
    h
}

/// What a message is known by in the index.
#[derive(Clone, Debug, Default, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
pub struct Key {
    /// The account, as its store names itself.
    pub account: String,
    /// `gm:` and Gmail's ID, `id:` and the `Message-ID`, or `h:` and the
    /// hash of what else tells it.
    pub id: String,
}

impl Key {
    /// The key of a message of `account`: by Gmail's ID for it, else its
    /// `Message-ID`, else its date, sender, subject and size.
    pub fn of(
        account: &str,
        gmail: Option<u64>,
        message_id: &str,
        date: Option<i64>,
        from: &str,
        subject: &str,
        size: u32,
    ) -> Key {
        let id = match gmail {
            Some(g) => format!("gm:{g}"),
            None if !message_id.is_empty() => format!("id:{message_id}"),
            None => {
                let told = format!("{}|{from}|{subject}|{size}", date.unwrap_or(0));
                format!("h:{:016x}", fnv(told.as_bytes()))
            }
        };
        Key {
            account: account.to_string(),
            id,
        }
    }
}

/// Where a message is on its server: a folder, as the folder was then, and
/// its UID there.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Place {
    pub folder: String,
    /// The folder's UIDVALIDITY when the UID was given.
    pub validity: u32,
    pub uid: u32,
}

/// What a folder was, as the index last saw it, to know what is new.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct FolderState {
    pub role: Option<Role>,
    /// What its path's parts are parted by.
    #[serde(default)]
    pub delimiter: Option<String>,
    pub validity: u32,
    /// The UID the next message will have, as it was then.
    pub next: u32,
    /// Its highest mod-sequence, where it keeps them.
    #[serde(default)]
    pub modseq: Option<u64>,
    /// The lowest UID fetched, as backfill goes down.
    #[serde(default)]
    pub lowest: Option<u32>,
    /// When it was last looked in, and when its every flag was last read,
    /// in seconds since 1970.
    #[serde(default)]
    pub synced: i64,
    #[serde(default)]
    pub flags_read: i64,
}

/// A message, as the index has it.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct Record {
    pub key: Key,
    pub places: Vec<Place>,
    /// The labels the server keeps on it: Gmail's, as it gives them, or
    /// its keywords.
    #[serde(default)]
    pub server: Vec<String>,
    #[serde(default)]
    pub message_id: String,
    /// Gmail's ID of its thread.
    #[serde(default)]
    pub thread: Option<u64>,
    /// The sender's name, else their address; and the address.
    pub from: String,
    pub address: String,
    /// The addresses it went to, and those copied.
    #[serde(default)]
    pub to: Vec<String>,
    #[serde(default)]
    pub cc: Vec<String>,
    /// Its `List-Id`, without the brackets.
    #[serde(default)]
    pub list: String,
    pub subject: String,
    pub date: Option<i64>,
    pub size: u32,
    /// The first words of its text, no more than `PREVIEW` bytes.
    #[serde(default)]
    pub preview: String,
    pub flags: Flags,
    /// When the index first saw it, in seconds since 1970.
    pub first_seen: i64,
}

/// A message as a server gives it to be indexed: its UID and flags, the
/// labels it keeps on it, Gmail's IDs, its size, and its header with the
/// start of its text.
#[derive(Clone, Debug, Default)]
pub struct Indexed {
    pub uid: u32,
    pub flags: Flags,
    pub labels: Vec<String>,
    pub gmail: Option<u64>,
    pub thread: Option<u64>,
    pub modseq: Option<u64>,
    pub size: u32,
    pub raw: Vec<u8>,
}

impl Record {
    /// The record of `indexed`, of `account`, in `folder` as it is at
    /// `validity`, first seen `now`.
    pub fn of(account: &str, folder: &str, validity: u32, indexed: &Indexed, now: i64) -> Record {
        let s = mail::summary(indexed.uid, indexed.flags, indexed.size, &indexed.raw);
        let parsed = MessageParser::default().parse(&indexed.raw);
        let addresses = |a: Option<&Address>| -> Vec<String> {
            a.map(|a| {
                a.iter()
                    .filter_map(|x| x.address())
                    .map(|x| mail::clean(x).to_lowercase())
                    .filter(|x| !x.is_empty())
                    .collect()
            })
            .unwrap_or_default()
        };
        let (to, cc, text) = match &parsed {
            Some(m) => (
                addresses(m.to()),
                addresses(m.cc()),
                m.body_text(0).map(|t| t.into_owned()).unwrap_or_default(),
            ),
            None => (Vec::new(), Vec::new(), String::new()),
        };
        let list = mail::field(&indexed.raw, "List-Id")
            .map(|l| list_id(&l))
            .unwrap_or_default();
        let key = Key::of(
            account,
            indexed.gmail,
            &s.message_id,
            s.date,
            &s.address,
            &s.subject,
            indexed.size,
        );
        Record {
            key,
            places: vec![Place {
                folder: folder.to_string(),
                validity,
                uid: indexed.uid,
            }],
            server: indexed.labels.clone(),
            message_id: s.message_id,
            thread: indexed.thread,
            from: s.from,
            address: s.address.to_lowercase(),
            to,
            cc,
            list,
            subject: s.subject,
            date: s.date,
            size: indexed.size,
            preview: cut(&mail::preview(&text, PREVIEW), PREVIEW),
            flags: indexed.flags,
            first_seen: now,
        }
    }

    /// Its place in `folder`, if it has one.
    pub fn place(&self, folder: &str) -> Option<&Place> {
        self.places.iter().find(|p| p.folder == folder)
    }
}

/// A `List-Id` as it is kept: what is in its brackets, else all of it.
fn list_id(value: &str) -> String {
    let inner = match (value.rfind('<'), value.rfind('>')) {
        (Some(a), Some(b)) if a < b => &value[a + 1..b],
        _ => value,
    };
    mail::clean(inner).to_lowercase()
}

/// `s`, no longer than `bytes`, cut where a character ends.
fn cut(s: &str, bytes: usize) -> String {
    if s.len() <= bytes {
        return s.to_string();
    }
    let mut end = bytes;
    while !s.is_char_boundary(end) {
        end -= 1;
    }
    s[..end].to_string()
}
