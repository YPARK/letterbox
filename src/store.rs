//! Where an account's mail is: an IMAP server, or the demo's mail in
//! memory. Each is asked the same few things, from one thread.

use crate::mail::{Folder, Summary};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Flag {
    Seen,
    Flagged,
}

impl Flag {
    pub fn imap(self) -> &'static str {
        match self {
            Flag::Seen => "\\Seen",
            Flag::Flagged => "\\Flagged",
        }
    }
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
}
