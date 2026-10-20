//! Each account's store, asked from a thread of its own, so the screen
//! never waits on a server. What is asked is answered in turn, but for
//! what the screen no longer wants: a message passed over while moving
//! through the list is not fetched, and folders are counted last.

use crate::mail::{self, Folder, Message, Summary};
use crate::store::{Flag, Page, Store};
use std::collections::VecDeque;
use std::sync::Arc;
use std::sync::mpsc::{self, Receiver, Sender};

pub enum Ask {
    Folders,
    Count(String),
    Page {
        folder: String,
        offset: usize,
        n: usize,
    },
    /// A message read whole, and marked read if `mark` is.
    Open {
        folder: String,
        uid: u32,
        mark: bool,
    },
    Set {
        folder: String,
        uid: u32,
        flag: Flag,
        on: bool,
    },
    Search {
        folder: String,
        words: String,
        n: usize,
    },
}

pub enum Answer {
    Folders(Result<Vec<Folder>, String>),
    Count {
        folder: String,
        counts: Result<(u32, u32), String>,
    },
    Page {
        folder: String,
        offset: usize,
        page: Result<Page, String>,
    },
    Opened {
        folder: String,
        uid: u32,
        message: Result<Arc<Message>, String>,
        /// Whether it was marked read.
        marked: bool,
    },
    Set {
        folder: String,
        uid: u32,
        flag: Flag,
        on: bool,
        result: Result<(), String>,
    },
    Found {
        folder: String,
        words: String,
        found: Result<Vec<Summary>, String>,
    },
    /// Asked, and passed over for what was asked after it.
    Skipped,
    /// An account added, signed in, with its folders; not a store's
    /// answer, but the form's.
    Added(Result<Added, String>),
}

pub struct Added {
    pub name: String,
    pub store: Box<dyn Store>,
    pub folders: Vec<Folder>,
}

pub struct Reply {
    pub account: usize,
    pub answer: Answer,
}

/// Why something failed, as it is shown: what a server says in it is a
/// stranger's, and an escape in it would otherwise reach the terminal.
pub fn text(e: anyhow::Error) -> String {
    mail::clean_lines(&format!("{e:#}"))
}

fn answer(store: &mut dyn Store, ask: Ask) -> Answer {
    match ask {
        Ask::Folders => Answer::Folders(store.folders().map_err(text)),
        Ask::Count(folder) => Answer::Count {
            counts: store.counts(&folder).map_err(text),
            folder,
        },
        Ask::Page { folder, offset, n } => Answer::Page {
            page: store.page(&folder, offset, n).map_err(text),
            folder,
            offset,
        },
        Ask::Open { folder, uid, mark } => {
            let message = store
                .raw(&folder, uid)
                .and_then(|raw| mail::message(uid, raw))
                .map(Arc::new)
                .map_err(text);
            let marked =
                mark && message.is_ok() && store.set(&folder, uid, Flag::Seen, true).is_ok();
            Answer::Opened {
                folder,
                uid,
                message,
                marked,
            }
        }
        Ask::Set {
            folder,
            uid,
            flag,
            on,
        } => Answer::Set {
            result: store.set(&folder, uid, flag, on).map_err(text),
            folder,
            uid,
            flag,
            on,
        },
        Ask::Search { folder, words, n } => Answer::Found {
            found: store.search(&folder, &words, n).map_err(text),
            folder,
            words,
        },
    }
}

/// Whether `ask` is passed over for one asked after it, still to answer:
/// a folder counted again, as when the server is slower than the refresh,
/// is counted once.
fn superseded(ask: &Ask, after: &VecDeque<Ask>) -> bool {
    match ask {
        Ask::Open { .. } => after.iter().any(|a| matches!(a, Ask::Open { .. })),
        Ask::Page { offset: 0, .. } => after
            .iter()
            .any(|a| matches!(a, Ask::Page { offset: 0, .. })),
        Ask::Count(folder) => after
            .iter()
            .any(|a| matches!(a, Ask::Count(f) if f == folder)),
        _ => false,
    }
}

/// Ask `store`, on a thread of its own, what is sent on the sender given
/// back; each ask is answered on `replies`, as the account `account`'s.
pub fn spawn(account: usize, mut store: Box<dyn Store>, replies: Sender<Reply>) -> Sender<Ask> {
    let (asks, rx): (Sender<Ask>, Receiver<Ask>) = mpsc::channel();
    std::thread::spawn(move || {
        let mut queue: VecDeque<Ask> = VecDeque::new();
        loop {
            if queue.is_empty() {
                match rx.recv() {
                    Ok(a) => queue.push_back(a),
                    Err(_) => return,
                }
            }
            queue.extend(rx.try_iter());
            let at = queue
                .iter()
                .position(|a| !matches!(a, Ask::Count(_)))
                .unwrap_or(0);
            let Some(ask) = queue.remove(at) else {
                continue;
            };
            let answer = if superseded(&ask, &queue) {
                Answer::Skipped
            } else {
                answer(store.as_mut(), ask)
            };
            if replies.send(Reply { account, answer }).is_err() {
                return;
            }
        }
    });
    asks
}
