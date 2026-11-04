//! Messages letterbox moved to Trash, and when, so `d` there can put one
//! back where it was, and `letterbox trash restore` all of a time. Only
//! those letterbox itself moved are recorded. Nothing is ever removed from
//! Trash for good by letterbox: Trash keeps it as long as its server does;
//! a record past a month is let go, the message left where it is.
//!
//! The file is `trash.toml` beside the vault, plain: it holds no secret,
//! only which message came from where.

use anyhow::Context;
use serde::{Deserialize, Serialize};
use std::path::PathBuf;

/// How far back what was thrown away as junk is told of, by J, in seconds.
pub const KEPT: i64 = 7 * 24 * 60 * 60;
/// How long a record of a message trashed is kept, in seconds: past it the
/// record is let go, never the message.
pub const RECORD: i64 = 30 * 24 * 60 * 60;

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct Entry {
    /// The account, by its address.
    pub account: String,
    /// The message's `Message-ID`, with the brackets; empty when it had none.
    #[serde(default)]
    pub message_id: String,
    /// The Trash folder it was moved to, by its path on the server.
    pub folder: String,
    /// The UID it has there.
    pub uid: u32,
    /// The folder it was moved from.
    pub origin: String,
    /// When it was moved, in seconds since 1970.
    pub at: i64,
}

#[derive(Default, Serialize, Deserialize)]
struct File {
    #[serde(default)]
    message: Vec<Entry>,
}

pub struct Ledger {
    path: PathBuf,
    entries: Vec<Entry>,
    /// Why the file could not be read. Nothing is written over it while this
    /// is set: a bad read must not replace the record with an empty one.
    fault: Option<String>,
    /// How many times the file was written, as tests count it.
    saves: usize,
    /// When the file was last changed, and how long it was, as it was read:
    /// read again only once either is not.
    seen: Option<(std::time::SystemTime, u64)>,
}

/// When the file at `path` was last changed, and how long it is.
fn stamp(path: &std::path::Path) -> Option<(std::time::SystemTime, u64)> {
    let m = std::fs::metadata(path).ok()?;
    Some((m.modified().ok()?, m.len()))
}

impl Ledger {
    /// The ledger at `path`, empty when there is no file yet. A file that
    /// cannot be read is kept as it is: nothing is written until it can.
    pub fn open(path: PathBuf) -> Self {
        let (entries, fault) = read(&path);
        Ledger {
            seen: stamp(&path),
            path,
            entries,
            fault,
            saves: 0,
        }
    }

    /// Read the file again before it is looked in, as another letterbox,
    /// or an agent's, may have recorded what it moved since; kept as it was
    /// if it cannot be read now.
    fn refresh(&mut self) {
        let now = stamp(&self.path);
        if self.fault.is_none()
            && now != self.seen
            && let (entries, None) = read(&self.path)
        {
            self.entries = entries;
            self.seen = now;
        }
    }

    /// Read the file again, as another may have changed it.
    fn fresh(&mut self) -> anyhow::Result<()> {
        (self.entries, self.fault) = read(&self.path);
        self.seen = stamp(&self.path);
        self.writable()
    }

    /// `$LETTERBOX_TRASH`, else `trash.toml` in the vault's directory.
    pub fn load() -> Self {
        Self::open(path())
    }

    /// Why the file could not be read, if it could not.
    pub fn fault(&self) -> Option<&str> {
        self.fault.as_deref()
    }

    /// Where this message was, if letterbox moved it to `folder`.
    pub fn origin(
        &mut self,
        account: &str,
        folder: &str,
        message_id: &str,
        uid: u32,
    ) -> Option<String> {
        self.refresh();
        self.entries
            .iter()
            .rev()
            .find(|e| {
                e.account == account
                    && e.folder == folder
                    && ((!message_id.is_empty() && e.message_id == message_id) || e.uid == uid)
            })
            .map(|e| e.origin.clone())
    }

    /// The Message-IDs of what letterbox moved to Trash for `account`.
    pub fn message_ids(&self, account: &str) -> std::collections::HashSet<String> {
        self.entries
            .iter()
            .filter(|e| e.account == account && !e.message_id.is_empty())
            .map(|e| e.message_id.clone())
            .collect()
    }

    /// Where the message of `account` by `message_id` is in a Trash, if
    /// letterbox moved it there.
    pub fn find(&mut self, account: &str, message_id: &str) -> Option<Entry> {
        if message_id.is_empty() {
            return None;
        }
        self.refresh();
        self.entries
            .iter()
            .rev()
            .find(|e| e.account == account && e.message_id == message_id)
            .cloned()
    }

    pub fn record(&mut self, entry: Entry) -> anyhow::Result<()> {
        self.record_many(vec![entry])
    }

    /// Record each of `entries`, written once.
    pub fn record_many(&mut self, entries: Vec<Entry>) -> anyhow::Result<()> {
        self.change(|all| {
            for entry in entries {
                all.retain(|e| {
                    !(e.account == entry.account && e.folder == entry.folder && e.uid == entry.uid)
                });
                all.push(entry);
            }
            true
        })
    }

    /// Forget the message `uid` in `folder`: it was put back, or removed.
    pub fn forget(&mut self, account: &str, folder: &str, uid: u32) -> anyhow::Result<()> {
        self.forget_many(&[(account.to_string(), folder.to_string(), uid)])
    }

    /// Those moved to Trash from `since` to `until`, both included, of
    /// `account` if one is named, in the order they were moved.
    pub fn between(&self, since: i64, until: i64, account: Option<&str>) -> Vec<Entry> {
        self.entries
            .iter()
            .filter(|e| (since..=until).contains(&e.at))
            .filter(|e| account.is_none_or(|a| e.account == a))
            .cloned()
            .collect()
    }

    /// Those moved to Trash before `now` by at least [`RECORD`], for
    /// `account`: their records to let go. None when the file could not be
    /// read.
    pub fn stale(&mut self, account: &str, now: i64) -> Vec<Entry> {
        if self.fault.is_some() {
            return Vec::new();
        }
        self.refresh();
        self.entries
            .iter()
            .filter(|e| e.account == account && now.saturating_sub(e.at) >= RECORD)
            .cloned()
            .collect()
    }

    /// Forget each of `gone`, by account, folder and UID, written once.
    pub fn forget_many(&mut self, gone: &[(String, String, u32)]) -> anyhow::Result<()> {
        self.change(|all| {
            let n = all.len();
            all.retain(|e| {
                !gone
                    .iter()
                    .any(|(a, f, u)| e.account == *a && e.folder == *f && e.uid == *u)
            });
            all.len() != n
        })
    }

    /// Change what is recorded as `change` does, held against another
    /// letterbox, or an agent's, changing it too: read again first, what
    /// they recorded kept, and written if `change` says it changed.
    fn change(&mut self, change: impl FnOnce(&mut Vec<Entry>) -> bool) -> anyhow::Result<()> {
        if let Some(dir) = self.path.parent() {
            std::fs::create_dir_all(dir)?;
        }
        let lock = self.path.with_extension("toml.lock");
        let held = std::fs::OpenOptions::new()
            .create(true)
            .truncate(false)
            .write(true)
            .open(&lock)
            .with_context(|| format!("cannot open {}", lock.display()))?;
        held.lock()
            .with_context(|| format!("cannot hold {}", lock.display()))?;
        self.fresh()?;
        if change(&mut self.entries) {
            self.save()?;
        }
        Ok(())
    }

    /// Date every entry as moved at `at`: a test, ageing one past seven days.
    #[cfg(test)]
    pub fn backdate(&mut self, at: i64) {
        for e in &mut self.entries {
            e.at = at;
        }
        let _ = self.save();
    }

    fn writable(&self) -> anyhow::Result<()> {
        if let Some(why) = &self.fault {
            anyhow::bail!("{why}");
        }
        Ok(())
    }

    /// Whether the message of `account` by `message_id` is recorded, as
    /// it was last read: a test's look, changing nothing.
    #[cfg(test)]
    pub fn records(&self, account: &str, message_id: &str) -> bool {
        self.entries
            .iter()
            .any(|e| e.account == account && e.message_id == message_id)
    }

    /// How many times the file was written.
    #[cfg(test)]
    pub fn saves(&self) -> usize {
        self.saves
    }

    fn save(&mut self) -> anyhow::Result<()> {
        self.writable()?;
        self.saves += 1;
        if let Some(dir) = self.path.parent() {
            std::fs::create_dir_all(dir)?;
        }
        let file = File {
            message: self.entries.clone(),
        };
        let text = toml::to_string_pretty(&file).context("cannot record what was trashed")?;
        let tmp = self.path.with_extension("toml.new");
        std::fs::write(&tmp, text).with_context(|| format!("cannot write {}", tmp.display()))?;
        std::fs::rename(&tmp, &self.path)
            .with_context(|| format!("cannot write {}", self.path.display()))?;
        self.seen = stamp(&self.path);
        Ok(())
    }
}

/// What the file at `path` records, and why it cannot be read, if not.
fn read(path: &std::path::Path) -> (Vec<Entry>, Option<String>) {
    match std::fs::read_to_string(path) {
        Ok(text) => match toml::from_str::<File>(&text) {
            Ok(file) => (file.message, None),
            Err(e) => (
                Vec::new(),
                Some(format!("cannot read {}: {e}", path.display())),
            ),
        },
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => (Vec::new(), None),
        Err(e) => (
            Vec::new(),
            Some(format!("cannot read {}: {e}", path.display())),
        ),
    }
}

/// `$LETTERBOX_TRASH`, else `trash.toml` beside the vault.
pub fn path() -> PathBuf {
    if let Some(p) = std::env::var_os("LETTERBOX_TRASH").filter(|p| !p.is_empty()) {
        return PathBuf::from(p);
    }
    crate::vault::path()
        .parent()
        .map(|d| d.join("trash.toml"))
        .unwrap_or_else(|| PathBuf::from("trash.toml"))
}

/// What putting one back did: done, or why not.
#[derive(Debug)]
pub struct Restored {
    pub entry: Entry,
    pub result: Result<(), String>,
}

/// Put each of `entries` back where it was moved from, through `store`,
/// and forget, once, those that are there. One is put back only once its
/// folder is known to have it: a move that failed, or whose new place was
/// not told, is put back if the folder it came from holds it now; else it
/// is said, and kept in the ledger as it was.
pub fn restore(
    ledger: &mut Ledger,
    entries: &[Entry],
    store: &mut dyn crate::store::Store,
) -> Vec<Restored> {
    let mut done: Vec<Restored> = entries
        .iter()
        .map(|entry| {
            let result = store
                .relocate(&entry.folder, entry.uid, &entry.origin, &entry.message_id)
                .map(|_| ())
                .or_else(|e| {
                    let there = !entry.message_id.is_empty()
                        && store
                            .holds(&entry.origin, &entry.message_id)
                            .is_ok_and(|held| held);
                    if there { Ok(()) } else { Err(format!("{e:#}")) }
                });
            Restored {
                entry: entry.clone(),
                result,
            }
        })
        .collect();
    let gone: Vec<(String, String, u32)> = done
        .iter()
        .filter(|r| r.result.is_ok())
        .map(|r| (r.entry.account.clone(), r.entry.folder.clone(), r.entry.uid))
        .collect();
    if let Err(e) = ledger.forget_many(&gone) {
        let said = format!("put back, but the record not written: {e:#}");
        for r in done.iter_mut().filter(|r| r.result.is_ok()) {
            r.result = Err(said.clone());
        }
    }
    done
}
