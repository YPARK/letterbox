//! Messages letterbox moved to Trash, and when, so `d` there can put one
//! back, and so one kept past seven days can be removed for good. Only
//! those letterbox itself moved are recorded, and only those are removed:
//! mail already in Trash is left as the server keeps it.
//!
//! The file is `trash.toml` beside the vault, plain: it holds no secret,
//! only which message came from where.

use anyhow::Context;
use serde::{Deserialize, Serialize};
use std::path::PathBuf;

/// How long a message letterbox trashed can be put back, in seconds.
pub const KEPT: i64 = 7 * 24 * 60 * 60;

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
}

impl Ledger {
    /// The ledger at `path`, empty when there is no file yet. A file that
    /// cannot be read is kept as it is: nothing is written until it can.
    pub fn open(path: PathBuf) -> Self {
        let (entries, fault) = match std::fs::read_to_string(&path) {
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
        };
        Ledger {
            path,
            entries,
            fault,
        }
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
        &self,
        account: &str,
        folder: &str,
        message_id: &str,
        uid: u32,
    ) -> Option<String> {
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

    pub fn record(&mut self, entry: Entry) -> anyhow::Result<()> {
        self.writable()?;
        self.entries.retain(|e| {
            !(e.account == entry.account && e.folder == entry.folder && e.uid == entry.uid)
        });
        self.entries.push(entry);
        self.save()
    }

    /// Forget the message `uid` in `folder`: it was put back, or removed.
    pub fn forget(&mut self, account: &str, folder: &str, uid: u32) -> anyhow::Result<()> {
        self.writable()?;
        let n = self.entries.len();
        self.entries
            .retain(|e| !(e.account == account && e.folder == folder && e.uid == uid));
        if self.entries.len() == n {
            return Ok(());
        }
        self.save()
    }

    /// Those moved to Trash before `now` by at least [`KEPT`], for `account`.
    /// None when the file could not be read: nothing is removed from a
    /// record that was not understood.
    pub fn due(&self, account: &str, now: i64) -> Vec<Entry> {
        if self.fault.is_some() {
            return Vec::new();
        }
        self.entries
            .iter()
            .filter(|e| e.account == account && now.saturating_sub(e.at) >= KEPT)
            .cloned()
            .collect()
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

    fn save(&mut self) -> anyhow::Result<()> {
        self.writable()?;
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
            .with_context(|| format!("cannot write {}", self.path.display()))
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
