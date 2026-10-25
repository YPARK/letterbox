//! Where an account's mail is: an IMAP server, or the demo's mail in
//! memory. Each is asked the same few things, from one thread.

use crate::mail::{Folder, Summary};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Flag {
    Seen,
    Flagged,
    Answered,
}

impl Flag {
    pub fn imap(self) -> &'static str {
        match self {
            Flag::Seen => "\\Seen",
            Flag::Flagged => "\\Flagged",
            Flag::Answered => "\\Answered",
        }
    }
}

/// What became of a message asked to be removed for good.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Removal {
    /// It was removed.
    Gone,
    /// It was not in the folder.
    Missing,
}

/// Some of a folder's messages, newest first, and how many it holds.
#[derive(Clone, Debug, Default)]
pub struct Page {
    pub summaries: Vec<Summary>,
    pub total: u32,
}

/// How a server keeps labels: as Gmail's, as keywords of any name, or in
/// folders alone.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Kind {
    Gmail,
    Keywords,
    Folders,
}

/// A folder as its server says it is: how many messages it holds, its
/// UIDVALIDITY, the UID its next message will have, and its highest
/// mod-sequence where it keeps them.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Status {
    pub messages: u32,
    pub validity: u32,
    pub next: u32,
    pub modseq: Option<u64>,
}

/// A message's flags and labels, as they now are.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Change {
    pub uid: u32,
    pub flags: crate::mail::Flags,
    pub labels: Vec<String>,
    pub modseq: Option<u64>,
}

/// What a store that cannot do something says.
pub fn unsupported(what: &str) -> anyhow::Error {
    anyhow::anyhow!("this account cannot {what}")
}

pub trait Store: Send {
    /// The folders, in a mail app's order.
    fn folders(&mut self) -> anyhow::Result<Vec<Folder>>;
    /// How many messages `folder` holds, and how many of them are unread.
    fn counts(&mut self, folder: &str) -> anyhow::Result<(u32, u32)>;
    /// `n` messages of `folder`, newest first, after the `offset` newest.
    fn page(&mut self, folder: &str, offset: usize, n: usize) -> anyhow::Result<Page>;
    /// A message's bytes, as sent; it is not marked read for being fetched.
    fn raw(&mut self, folder: &str, uid: u32) -> anyhow::Result<Vec<u8>>;
    fn set(&mut self, folder: &str, uid: u32, flag: Flag, on: bool) -> anyhow::Result<()>;
    /// The newest `n` messages of `folder` with `words` in them, newest first.
    fn search(&mut self, folder: &str, words: &str, n: usize) -> anyhow::Result<Vec<Summary>>;
    /// Move the message `uid` from `from` to `to`. The UID it has in `to`.
    /// `message_id` is how that UID is found once it has moved; when it is
    /// empty the header is read. An error may mean it was moved and its new
    /// UID could not be told, so nothing should be recorded of it.
    fn relocate(&mut self, from: &str, uid: u32, to: &str, message_id: &str)
    -> anyhow::Result<u32>;
    /// Remove the message `uid` from `folder`, for good.
    fn expunge(&mut self, folder: &str, uid: u32) -> anyhow::Result<Removal>;
    /// The folder this account keeps as Trash, when one was chosen.
    fn trash(&self) -> Option<&str> {
        None
    }
    /// Add `raw` to the end of `folder`, seen if `seen` is.
    fn append(&mut self, folder: &str, raw: &[u8], seen: bool) -> anyhow::Result<()>;
    /// Make a folder named `path`.
    fn create(&mut self, path: &str) -> anyhow::Result<()>;
    /// The address mail from this account is sent as.
    fn address(&self) -> &str;
    /// Send `raw`. What to say of it: that it was sent, or, for the demo,
    /// that it was only kept.
    fn send(&mut self, raw: &[u8]) -> anyhow::Result<String>;

    // What the map reads, from a connection of its own.

    /// Another store of the same mail, on a connection of its own: the
    /// map's, so the screen's is never kept waiting by it.
    fn twin(&self) -> Option<Box<dyn Store>> {
        None
    }
    /// What the account is known by in the index: the same however
    /// letterbox calls it.
    fn ident(&self) -> String {
        self.address().to_string()
    }
    /// How the server keeps labels.
    fn kind(&mut self) -> anyhow::Result<Kind> {
        Err(unsupported("say how it keeps labels"))
    }
    /// The folder, as its server now says it is.
    fn status(&mut self, _folder: &str) -> anyhow::Result<Status> {
        Err(unsupported("say how a folder is"))
    }
    /// The messages `uids` of `folder`, as the index reads them.
    fn fetch_index(
        &mut self,
        _folder: &str,
        _uids: &[u32],
    ) -> anyhow::Result<Vec<crate::index::record::Indexed>> {
        Err(unsupported("give its messages to the index"))
    }
    /// The flags and labels of the messages of `folder` changed since the
    /// mod-sequence `since`; or, without one, of the messages `uids`.
    fn changed(
        &mut self,
        _folder: &str,
        _since: Option<u64>,
        _uids: &[u32],
    ) -> anyhow::Result<Vec<Change>> {
        Err(unsupported("say what changed"))
    }
    /// Every UID in `folder`, lowest first.
    fn uids(&mut self, _folder: &str) -> anyhow::Result<Vec<u32>> {
        Err(unsupported("list a folder's UIDs"))
    }
}
