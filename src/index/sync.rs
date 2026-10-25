//! Each account's mail read into the index, on a thread and a connection
//! of its own, so that the screen's connection is never kept waiting by
//! it. A pass looks in every folder mapped, the Inbox first, then All Mail
//! or Archive, then Sent, then the rest: what has gone is let go, what is
//! new is read, and flags read again; then what is older, newest first, a
//! hundred messages at a time, until all of it is. Junk, Trash and Drafts
//! are not mapped, and on Gmail All Mail alone is, as it holds the rest.

use super::{Cached, FolderState, Index, Key, Record};
use crate::mail::{Folder, Role};
use crate::store::{Kind, Store};
use crate::worker::{Answer, Reply};
use std::collections::BTreeMap;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{self, Receiver, RecvTimeoutError, Sender};
use std::sync::{Arc, Mutex};
use std::time::Duration;

/// How many messages are read at a time.
pub const CHUNK: usize = 100;
/// How many of a folder's newest messages have their flags read each pass
/// where the server keeps no mod-sequences; all of them are, once a day.
const NEWEST: usize = 500;
const DAY: i64 = 24 * 60 * 60;
/// How often what the index holds is written down while the sync waits.
const TICK: Duration = Duration::from_secs(5);

/// What a pass did: how many messages it read, and whether it looked in
/// every folder and read all there was.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Passed {
    pub read: usize,
    pub complete: bool,
}

/// What the sync is asked: to pass over the account now.
pub enum Ask {
    Pass,
}

/// Folders that hold no mail, as Microsoft's servers show a calendar, the
/// contacts and the like, and Apple's its notes, by their top folder's
/// name, in lower case.
const NOT_MAIL: &[&str] = &[
    "calendar",
    "contacts",
    "tasks",
    "notes",
    "journal",
    "sync issues",
    "conversation history",
    "rss feeds",
    "rss subscriptions",
    "outbox",
];

/// Whether `folder` holds no mail, or is under one that holds none.
fn not_mail(folder: &Folder) -> bool {
    let top = match folder.delimiter.as_deref().filter(|d| !d.is_empty()) {
        Some(d) => folder.path.split(d).next().unwrap_or(&folder.path),
        None => folder.path.split(['/', '.']).next().unwrap_or(&folder.path),
    };
    NOT_MAIL.contains(&top.to_lowercase().as_str())
}

/// The folders of `folders` the map reads, in the order it reads them:
/// every one but Junk, Trash and Drafts, and those that hold no mail; on
/// Gmail, All Mail alone.
pub fn mapped(folders: &[Folder], kind: Kind) -> Vec<&Folder> {
    if kind == Kind::Gmail
        && let Some(all) = folders.iter().find(|f| f.all && f.selectable)
    {
        return vec![all];
    }
    let mut out: Vec<&Folder> = folders
        .iter()
        .filter(|f| f.selectable && !matches!(f.role, Role::Junk | Role::Trash | Role::Drafts))
        .filter(|f| !not_mail(f))
        .collect();
    out.sort_by_key(|f| match f.role {
        Role::Inbox => 0,
        _ if f.all => 1,
        Role::Archive => 1,
        Role::Sent => 2,
        _ => 3,
    });
    out
}

/// Read the messages `keys` of this store's account again, as the index
/// now reads a message, where each is and its labels kept. How many were.
pub fn refresh(
    store: &mut dyn Store,
    index: &Mutex<Index>,
    keys: &[Key],
    now: i64,
) -> anyhow::Result<usize> {
    let account = store.ident();
    let mut by_folder: BTreeMap<String, Vec<(u32, Record)>> = BTreeMap::new();
    {
        let index = lock(index);
        for key in keys.iter().filter(|k| k.account == account) {
            let Some(r) = index.records.get(key) else {
                continue;
            };
            if let Some(p) = r.places.first() {
                by_folder
                    .entry(p.folder.clone())
                    .or_default()
                    .push((p.uid, r.clone()));
            }
        }
    }
    let mut events = Vec::new();
    for (folder, old) in by_folder {
        let uids: Vec<u32> = old.iter().map(|(u, _)| *u).collect();
        for chunk in uids.chunks(100) {
            for got in store.fetch_index(&folder, chunk)? {
                let Some((_, was)) = old.iter().find(|(u, _)| *u == got.uid) else {
                    continue;
                };
                let mut r = Record::of(&account, &folder, 0, &got, now);
                if r.key != was.key {
                    continue;
                }
                // Its places as they were, each folder's validity with it.
                r.places = was.places.clone();
                r.server = was.server.clone();
                events.push(Cached::Upsert(Box::new(r)));
            }
        }
    }
    let n = events.len();
    apply(index, events)?;
    Ok(n)
}

fn lock(index: &Mutex<Index>) -> std::sync::MutexGuard<'_, Index> {
    index.lock().unwrap_or_else(|e| e.into_inner())
}

/// Take `events` into the index, which is locked for them alone.
fn apply(index: &Mutex<Index>, events: Vec<Cached>) -> anyhow::Result<()> {
    if events.is_empty() {
        return Ok(());
    }
    lock(index).cache(events)
}

/// The messages of `account` the index has in `folder`, by their UIDs.
fn placed(index: &Mutex<Index>, account: &str, folder: &str) -> BTreeMap<u32, Key> {
    let index = lock(index);
    index
        .records
        .values()
        .filter(|r| r.key.account == account)
        .filter_map(|r| Some((r.place(folder)?.uid, r.key.clone())))
        .collect()
}

/// A folder looked in, and what of it is still to read, newest first.
struct Todo {
    path: String,
    state: FolderState,
    older: Vec<u32>,
}

/// Pass over the account of `store` once: look in each folder mapped,
/// read what is new, then what is older, saying how far it has got by
/// `say`; `stop` stops it between one batch and the next. Once every
/// folder is read whole, a message in none of them is forgotten.
pub fn pass(
    store: &mut dyn Store,
    index: &Mutex<Index>,
    now: i64,
    stop: &AtomicBool,
    say: &mut dyn FnMut(String),
) -> anyhow::Result<Passed> {
    let account = store.ident();
    let kind = store.kind().unwrap_or(Kind::Folders);
    let folders = store.folders()?;
    let mapped = mapped(&folders, kind);
    let mut complete = true;
    // Folders gone, or no longer mapped: what was in them is let go.
    let known: Vec<String> = lock(index)
        .folders
        .keys()
        .filter(|(a, _)| *a == account)
        .map(|(_, f)| f.clone())
        .collect();
    let gone: Vec<Cached> = known
        .into_iter()
        .filter(|f| !mapped.iter().any(|m| m.path == *f))
        .map(|folder| Cached::Reset {
            account: account.clone(),
            folder,
        })
        .collect();
    apply(index, gone)?;
    let mut todos = Vec::new();
    for folder in &mapped {
        match look(store, index, &account, folder, now) {
            Ok(todo) => todos.push(todo),
            Err(e) => {
                complete = false;
                say(format!("cannot read {}: {e:#}", folder.name));
            }
        }
    }
    let total: usize = todos.iter().map(|t| t.older.len()).sum();
    let mut read = 0;
    'folders: for todo in &mut todos {
        let older = std::mem::take(&mut todo.older);
        for chunk in older.chunks(CHUNK) {
            if stop.load(Ordering::Relaxed) {
                complete = false;
                break 'folders;
            }
            let fetched = match store.fetch_index(&todo.path, chunk) {
                Ok(f) => f,
                Err(e) => {
                    complete = false;
                    say(format!("cannot read {}: {e:#}", todo.path));
                    continue 'folders;
                }
            };
            let mut events: Vec<Cached> = fetched
                .iter()
                .map(|i| {
                    let r = Record::of(&account, &todo.path, todo.state.validity, i, now);
                    Cached::Upsert(Box::new(r))
                })
                .collect();
            if let Some(&low) = chunk.iter().min() {
                todo.state.lowest = Some(todo.state.lowest.map_or(low, |l| l.min(low)));
            }
            events.push(Cached::Folder {
                account: account.clone(),
                folder: todo.path.clone(),
                state: todo.state.clone(),
            });
            apply(index, events)?;
            read += chunk.len();
            say(format!("mapping: {read} of {total} messages read"));
        }
    }
    if complete {
        let forgotten: Vec<Cached> = lock(index)
            .records
            .values()
            .filter(|r| r.key.account == account && r.places.is_empty())
            .map(|r| Cached::Forget { key: r.key.clone() })
            .collect();
        apply(index, forgotten)?;
    }
    // Labels given that the server now has are no longer pending.
    let mut index = lock(index);
    let settled = index.settled(&account, now);
    if !settled.is_empty() {
        index.keep(settled)?;
    }
    Ok(Passed { read, complete })
}

/// Look in `folder`: let go of what has left it, read what is new and the
/// flags of what was there; and say what is older, still to read.
fn look(
    store: &mut dyn Store,
    index: &Mutex<Index>,
    account: &str,
    folder: &Folder,
    now: i64,
) -> anyhow::Result<Todo> {
    let path = folder.path.as_str();
    let status = store.status(path)?;
    let old = lock(index)
        .folders
        .get(&(account.to_string(), path.to_string()))
        .cloned();
    let mut events = Vec::new();
    // Its UIDs are another folder's now, as UIDVALIDITY says: read again.
    let fresh = old.as_ref().is_none_or(|o| o.validity != status.validity);
    let mut state = match old {
        Some(old) if !fresh => old,
        old => {
            if old.is_some() {
                events.push(Cached::Reset {
                    account: account.to_string(),
                    folder: path.to_string(),
                });
            }
            FolderState {
                validity: status.validity,
                ..FolderState::default()
            }
        }
    };
    state.role = Some(folder.role);
    state.all = folder.all;
    state.delimiter = folder.delimiter.clone();
    let on_server = store.uids(path)?;
    let placed = if fresh {
        BTreeMap::new()
    } else {
        placed(index, account, path)
    };
    let here: std::collections::BTreeSet<u32> = on_server.iter().copied().collect();
    for (uid, key) in &placed {
        if !here.contains(uid) {
            events.push(Cached::Unplace {
                key: key.clone(),
                folder: path.to_string(),
                uid: *uid,
            });
        }
    }
    let missing: Vec<u32> = on_server
        .iter()
        .copied()
        .filter(|u| !placed.contains_key(u))
        .collect();
    let (new, older): (Vec<u32>, Vec<u32>) = missing
        .into_iter()
        .partition(|&u| !fresh && state.next > 0 && u >= state.next);
    if !fresh {
        for c in flags(store, path, &status, &mut state, &placed, now)? {
            if let Some(key) = placed.get(&c.uid) {
                events.push(Cached::Flags {
                    key: key.clone(),
                    flags: c.flags,
                });
                events.push(Cached::Labels {
                    key: key.clone(),
                    labels: c.labels,
                });
            }
        }
    }
    apply(index, std::mem::take(&mut events))?;
    for chunk in new.rchunks(CHUNK) {
        let fetched = store.fetch_index(path, chunk)?;
        let upserts = fetched
            .iter()
            .map(|i| Cached::Upsert(Box::new(Record::of(account, path, state.validity, i, now))))
            .collect();
        apply(index, upserts)?;
    }
    state.next = status.next;
    state.modseq = status.modseq;
    state.synced = now;
    apply(
        index,
        vec![Cached::Folder {
            account: account.to_string(),
            folder: path.to_string(),
            state: state.clone(),
        }],
    )?;
    let mut older = older;
    older.reverse();
    Ok(Todo {
        path: path.to_string(),
        state,
        older,
    })
}

/// The flags and labels of what the index has of the folder, as they now
/// are: those changed since its mod-sequence, where it keeps them; else
/// the newest few hundred's, and everyone's once a day.
fn flags(
    store: &mut dyn Store,
    path: &str,
    status: &crate::store::Status,
    state: &mut FolderState,
    placed: &BTreeMap<u32, Key>,
    now: i64,
) -> anyhow::Result<Vec<crate::store::Change>> {
    if let (Some(was), Some(is)) = (state.modseq, status.modseq) {
        if was == is {
            return Ok(Vec::new());
        }
        return store.changed(path, Some(was), &[]);
    }
    let uids: Vec<u32> = if now - state.flags_read >= DAY {
        state.flags_read = now;
        placed.keys().copied().collect()
    } else {
        placed.keys().rev().take(NEWEST).copied().collect()
    };
    let mut all = Vec::new();
    for chunk in uids.chunks(NEWEST) {
        all.extend(store.changed(path, None, chunk)?);
    }
    Ok(all)
}

/// Pass over the account of `store` on a thread of its own, each time it
/// is asked, saying how far it is on `replies` as the account `account`'s;
/// what the index holds is written down while it waits. It ends once the
/// sender given back is dropped, its last pass stopped where it was.
pub fn spawn(
    account: usize,
    mut store: Box<dyn Store>,
    index: Arc<Mutex<Index>>,
    replies: Sender<Reply>,
) -> (Sender<Ask>, Arc<AtomicBool>) {
    let (asks, rx): (Sender<Ask>, Receiver<Ask>) = mpsc::channel();
    let stop = Arc::new(AtomicBool::new(false));
    let stopping = stop.clone();
    std::thread::spawn(move || {
        loop {
            match rx.recv_timeout(TICK) {
                Ok(Ask::Pass) => {}
                Err(RecvTimeoutError::Timeout) => {
                    let _ = lock(&index).tick();
                    continue;
                }
                Err(RecvTimeoutError::Disconnected) => break,
            }
            // Passes asked while one waited are the one pass.
            for Ask::Pass in rx.try_iter() {
                let _ = replies.send(Reply {
                    account,
                    answer: Answer::Skipped,
                });
            }
            let progress = replies.clone();
            let mut say = |said: String| {
                let _ = progress.send(Reply {
                    account,
                    answer: Answer::Mapping(said),
                });
            };
            let now = jiff::Timestamp::now().as_second();
            let passed =
                pass(store.as_mut(), &index, now, &stopping, &mut say).map_err(crate::worker::text);
            let _ = lock(&index).tick();
            if replies
                .send(Reply {
                    account,
                    answer: Answer::Mapped(passed),
                })
                .is_err()
            {
                break;
            }
        }
        let _ = lock(&index).flush();
    });
    (asks, stop)
}
