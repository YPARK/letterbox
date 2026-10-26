//! Each account's store, asked from a thread of its own, so the screen
//! never waits on a server. What is asked is answered in turn, but for
//! what the screen no longer wants: a message passed over while moving
//! through the list is not fetched, and folders are counted last.

use crate::mail::{self, Folder, Message, Summary};
use crate::store::{Flag, Page, Removal, Store};
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
    /// Move a message. `trashed` records where it came from, so it can be
    /// put back; a move out of Trash does not.
    Move {
        from: String,
        uid: u32,
        to: String,
        message_id: String,
        trashed: bool,
    },
    Expunge {
        folder: String,
        uid: u32,
    },
    /// File a message under a label: given it, and out of the Inbox. `key` is
    /// what the index knows it by.
    File {
        folder: String,
        uid: u32,
        label: String,
        message_id: String,
        key: crate::index::Key,
    },
    /// Call a label something else, on the server.
    RenameLabel {
        old: String,
        new: String,
    },
    /// Take a message out of the Inbox, given no label.
    Archive {
        folder: String,
        uid: u32,
        message_id: String,
    },
    /// Send `raw`. `answer` is the message it replies to, marked answered
    /// once the send is taken.
    Send {
        raw: Vec<u8>,
        answer: Option<(String, u32)>,
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
    /// The account's address, sent once the store's thread starts, before
    /// anything is asked: the ledger is kept by the address.
    Address(String),
    Moved {
        from: String,
        uid: u32,
        to: String,
        message_id: String,
        trashed: bool,
        new_uid: Result<u32, String>,
        email: String,
    },
    Expunged {
        folder: String,
        uid: u32,
        result: Result<Removal, String>,
    },
    Sent {
        said: Result<String, String>,
        answer: Option<(String, u32)>,
    },
    /// Asked, and passed over for what was asked after it.
    Skipped,
    /// An account added, signed in, with its folders; not a store's
    /// answer, but the form's.
    Added(Result<Added, String>),
    /// How to sign in on Microsoft's pages, while the form waits.
    Waiting(crate::oauth::Waiting),
    /// Whether Microsoft says an address signs in with it.
    Realm {
        email: String,
        microsoft: bool,
    },
    /// A linked document downloaded, and where it is kept.
    Fetched(Result<std::path::PathBuf, String>),
    /// The pages of the message `uid`, typeset.
    #[cfg(feature = "pages")]
    Paged {
        uid: u32,
        pages: Result<Vec<image::RgbaImage>, String>,
    },
    /// Where each paper of the message read stands with paperstack.
    Standings(Result<Vec<crate::paperstack::Standing>, String>),
    /// The paper `at` of those listed, put in paperstack, starred.
    Shelved {
        at: usize,
        found: Result<crate::paperstack::Found, String>,
    },
    /// How far the map's sync of the account has got, as it goes; not an
    /// answer to what was asked.
    Mapping(String),
    /// A pass of the map's sync over the account, done.
    Mapped(Result<crate::index::sync::Passed, String>),
    Archived(Result<(), String>),
    Filed {
        key: crate::index::Key,
        label: String,
        filed: Result<crate::store::Filed, String>,
    },
    LabelRenamed {
        old: String,
        result: Result<(), String>,
    },
    /// The map laid out again, by the lens given: where each message, by
    /// its key, would be; not a store's answer.
    Relaid {
        lens: crate::index::Lens,
        keys: Vec<crate::index::Key>,
        at: Vec<crate::model::Point>,
    },
    /// The map made again from the index; not a store's answer.
    Drawn(Box<crate::map::Snapshot>),
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
        Ask::Move {
            from,
            uid,
            to,
            message_id,
            trashed,
        } => {
            let email = store.address().to_string();
            let new_uid = store.relocate(&from, uid, &to, &message_id).map_err(text);
            Answer::Moved {
                new_uid,
                email,
                from,
                uid,
                to,
                message_id,
                trashed,
            }
        }
        Ask::File {
            folder,
            uid,
            label,
            message_id,
            key,
        } => Answer::Filed {
            filed: store.file(&folder, uid, &label, &message_id).map_err(text),
            key,
            label,
        },
        Ask::RenameLabel { old, new } => Answer::LabelRenamed {
            result: store.rename_label(&old, &new).map_err(text),
            old,
        },
        Ask::Archive {
            folder,
            uid,
            message_id,
        } => Answer::Archived(store.archive(&folder, uid, &message_id).map_err(text)),
        Ask::Expunge { folder, uid } => Answer::Expunged {
            result: store.expunge(&folder, uid).map_err(text),
            folder,
            uid,
        },
        Ask::Send { raw, answer } => {
            let said = store.send(&raw).map_err(text);
            if said.is_ok()
                && let Some((folder, uid)) = &answer
            {
                let _ = store.set(folder, *uid, Flag::Answered, true);
            }
            Answer::Sent { said, answer }
        }
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
        let _ = replies.send(Reply {
            account,
            answer: Answer::Address(store.address().to_string()),
        });
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
