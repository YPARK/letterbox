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
}
