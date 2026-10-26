//! The index: every message of every account, as the map reads them, kept
//! beside the vault in two journals encrypted to the map's key. `cache` is
//! what the servers say, and can be made again from them; `keep` is what
//! only letterbox knows, and cannot: the labels it alone keeps, every "not
//! it", where each message is on the map, and the labels given and not
//! yet confirmed by a server. The app and each account's sync share one
//! index, locked only to take in a batch.

pub mod journal;
pub mod key;
pub mod labels;
pub mod record;
pub mod sync;

use crate::mail::Flags;
use crate::secret::Keeper;
use age::x25519::Identity;
use journal::Journal;
pub use record::{FolderState, Key, Place, Record};
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, BTreeSet};
use std::path::Path;

/// What the servers say, as it is written down: each event a change.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(tag = "event", rename_all = "lowercase")]
pub enum Cached {
    /// A message, as fetched: what it says is taken, and its place added
    /// to those it had.
    Upsert(Box<Record>),
    Place {
        key: Key,
        place: Place,
    },
    Unplace {
        key: Key,
        folder: String,
        uid: u32,
    },
    Flags {
        key: Key,
        flags: Flags,
    },
    /// The labels the server keeps on it, all of them.
    Labels {
        key: Key,
        labels: Vec<String>,
    },
    Folder {
        account: String,
        folder: String,
        state: FolderState,
    },
    /// The folder's UIDs mean nothing now: every place in it is let go.
    Reset {
        account: String,
        folder: String,
    },
    Forget {
        key: Key,
    },
}

/// How the map is laid out: by what mail says and who it is from, or by
/// either alone.
#[derive(
    Clone, Copy, Debug, Default, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize,
)]
#[serde(rename_all = "lowercase")]
pub enum Lens {
    #[default]
    Both,
    Words,
    People,
}

impl Lens {
    pub const ALL: [Lens; 3] = [Lens::Both, Lens::Words, Lens::People];

    pub fn name(self) -> &'static str {
        match self {
            Lens::Both => "words and people",
            Lens::Words => "words",
            Lens::People => "people",
        }
    }
}

/// What only letterbox knows, as it is written down.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(tag = "event", rename_all = "lowercase")]
pub enum Kept {
    /// A label letterbox alone keeps on a message, given or taken off.
    Label { key: Key, label: String, on: bool },
    /// The message is not one for the label, as its suggestion was turned
    /// down.
    NotIt { key: Key, label: String },
    /// Where each message is on the map, by the lens.
    Layout {
        lens: Lens,
        at: Vec<(Key, f32, f32)>,
    },
    /// A label given or taken off, and not yet seen on the server.
    Pending {
        key: Key,
        label: String,
        on: bool,
        at: i64,
    },
    /// The server has it as it was asked: no longer pending.
    Settled { key: Key, label: String },
    /// When the map was last looked at.
    Looked { at: i64 },
    /// A label called something else, wherever letterbox alone keeps it.
    Renamed { from: String, to: String },
}

/// A label given or taken off, waiting on the server.
#[derive(Clone, Debug, PartialEq)]
pub struct Waiting {
    pub label: String,
    pub on: bool,
    pub at: i64,
}

/// A label as labels are matched: in lower case, whatever case it was
/// given in.
pub fn fold(label: &str) -> String {
    label.trim().to_lowercase()
}

/// What only letterbox knows, as it now is.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Keep {
    /// The labels letterbox alone keeps on each message, as given.
    pub labels: BTreeMap<Key, Vec<String>>,
    /// Each message and label, folded, it was said not to be one for.
    pub not_it: BTreeSet<(Key, String)>,
    pub layouts: BTreeMap<Lens, BTreeMap<Key, (f32, f32)>>,
    /// What waits on the servers, by message and label, folded.
    pub pending: BTreeMap<(Key, String), Waiting>,
    pub looked: i64,
}

impl Keep {
    fn apply(&mut self, e: &Kept) {
        match e {
            Kept::Label { key, label, on } => {
                let labels = self.labels.entry(key.clone()).or_default();
                labels.retain(|l| fold(l) != fold(label));
                if *on {
                    labels.push(label.clone());
                }
                if labels.is_empty() {
                    self.labels.remove(key);
                }
            }
            Kept::NotIt { key, label } => {
                self.not_it.insert((key.clone(), fold(label)));
            }
            Kept::Layout { lens, at } => {
                self.layouts.insert(
                    *lens,
                    at.iter().map(|(k, x, y)| (k.clone(), (*x, *y))).collect(),
                );
            }
            Kept::Pending { key, label, on, at } => {
                self.pending.insert(
                    (key.clone(), fold(label)),
                    Waiting {
                        label: label.clone(),
                        on: *on,
                        at: *at,
                    },
                );
            }
            Kept::Settled { key, label } => {
                self.pending.remove(&(key.clone(), fold(label)));
            }
            Kept::Looked { at } => self.looked = *at,
            Kept::Renamed { from, to } => {
                let (from, folded) = (from.as_str(), fold(to));
                for labels in self.labels.values_mut() {
                    for l in labels.iter_mut().filter(|l| fold(l) == fold(from)) {
                        *l = to.clone();
                    }
                    let mut seen = BTreeSet::new();
                    labels.retain(|l| seen.insert(fold(l)));
                }
                self.not_it = std::mem::take(&mut self.not_it)
                    .into_iter()
                    .map(|(k, l)| {
                        if l == fold(from) {
                            (k, folded.clone())
                        } else {
                            (k, l)
                        }
                    })
                    .collect();
                self.pending = std::mem::take(&mut self.pending)
                    .into_iter()
                    .map(|((k, l), mut w)| {
                        if l == fold(from) {
                            w.label = to.clone();
                            ((k, folded.clone()), w)
                        } else {
                            ((k, l), w)
                        }
                    })
                    .collect();
            }
        }
    }

    /// Everything it holds, as events to write a base of.
    fn events(&self) -> Vec<Kept> {
        let mut all = Vec::new();
        for (key, labels) in &self.labels {
            for label in labels {
                all.push(Kept::Label {
                    key: key.clone(),
                    label: label.clone(),
                    on: true,
                });
            }
        }
        for (key, label) in &self.not_it {
            all.push(Kept::NotIt {
                key: key.clone(),
                label: label.clone(),
            });
        }
        for (lens, at) in &self.layouts {
            all.push(Kept::Layout {
                lens: *lens,
                at: at.iter().map(|(k, &(x, y))| (k.clone(), x, y)).collect(),
            });
        }
        for ((key, _), w) in &self.pending {
            all.push(Kept::Pending {
                key: key.clone(),
                label: w.label.clone(),
                on: w.on,
                at: w.at,
            });
        }
        if self.looked != 0 {
            all.push(Kept::Looked { at: self.looked });
        }
        all
    }
}

pub struct Index {
    pub records: BTreeMap<Key, Record>,
    /// Each folder as it was last looked in, by account and path.
    pub folders: BTreeMap<(String, String), FolderState>,
    pub keep: Keep,
    cache: Option<Journal<Cached>>,
    kept: Option<Journal<Kept>>,
    /// What you throw away, as learnt, and its journal.
    pub filter: crate::filter::Filter,
    taught: Option<Journal<crate::filter::Taught>>,
    /// What changes with every event taken in, so that what is drawn from
    /// the index knows to look again.
    pub version: u64,
    /// What went wrong reading the journals, to say on the screen.
    pub fault: Option<String>,
    /// Held while it is written to: one letterbox writes it at a time.
    held: Option<std::fs::File>,
}

/// The index's lock, as asked for.
enum Hold {
    /// Held, by the file it is, or by none where it cannot be made.
    Held(Option<std::fs::File>),
    /// Held by another.
    Busy,
}

/// Hold the lock of the index in `dir`, if no other letterbox does.
fn hold(dir: &Path) -> Hold {
    let file = std::fs::OpenOptions::new()
        .create(true)
        .truncate(false)
        .write(true)
        .open(dir.join("lock"));
    let Ok(file) = file else {
        return Hold::Held(None);
    };
    match file.try_lock() {
        Ok(()) => Hold::Held(Some(file)),
        Err(std::fs::TryLockError::WouldBlock) => Hold::Busy,
        Err(_) => Hold::Held(None),
    }
}

impl Index {
    /// An index in memory alone, as for the demo: nothing is written.
    pub fn memory() -> Index {
        Index {
            records: BTreeMap::new(),
            folders: BTreeMap::new(),
            keep: Keep::default(),
            cache: None,
            kept: None,
            filter: crate::filter::Filter::default(),
            taught: None,
            version: 0,
            fault: None,
            held: None,
        }
    }

    /// The index in `dir`, read with `identity`. A cache that cannot be
    /// read is begun again, as the servers will fill it; a `keep` that
    /// cannot be read is kept as it is, and nothing is written to it.
    /// Journals with many logs are written again as one base each.
    pub fn open_with(dir: &Path, identity: &Identity) -> Index {
        Self::opened(dir, identity, true)
    }

    /// The index in `dir`, read with `identity`, as `open_with` reads it,
    /// and nothing ever written: for another program to read while
    /// letterbox writes it.
    pub fn read_only(dir: &Path, identity: &Identity) -> Index {
        Self::opened(dir, identity, false)
    }

    /// The index in `dir` read, its journals kept to be written to if
    /// `write`.
    fn opened(dir: &Path, identity: &Identity, write: bool) -> Index {
        let recipient = identity.to_public();
        let (cache, cached) = Journal::open(dir, "cache", recipient.clone(), Some(identity));
        let (kept, keeps) = Journal::open(dir, "keep", recipient.clone(), Some(identity));
        let (taught, decisions) = Journal::open(dir, "taught", recipient, Some(identity));
        let mut index = Index::memory();
        for t in decisions {
            index.filter.learn(t);
        }
        let mut faults = Vec::new();
        if let Some(why) = cache.fault() {
            faults.push(format!("{why}; the map is made again from the servers"));
        } else {
            for e in &cached {
                index.take(e);
            }
        }
        for e in &keeps {
            index.keep.apply(e);
        }
        if let Some(why) = kept.fault() {
            faults.push(format!("{why}; nothing is written to it until it can be"));
        }
        if let Some(why) = taught.fault() {
            faults.push(format!("{why}; nothing more is learnt until it can be"));
        }
        // Written by another letterbox, as open: read, not written.
        let held = match write.then(|| hold(dir)) {
            Some(Hold::Busy) => {
                faults.push(
                    "open in another letterbox, which writes it; nothing is written here".into(),
                );
                None
            }
            Some(Hold::Held(file)) => Some(file),
            None => None,
        };
        let write = write && held.is_some();
        index.held = held.flatten();
        index.fault = (!faults.is_empty()).then(|| faults.join("; "));
        if write {
            index.cache = Some(cache);
            index.kept = Some(kept);
            index.taught = Some(taught);
            index.compact();
        }
        index
    }

    /// Whether what is new is written down: not in memory alone, nor read
    /// while another letterbox writes it.
    pub fn writes(&self) -> bool {
        self.taught.is_some()
    }

    /// The index in `dir` read only, as `read_only` reads it, its key found
    /// where `map.toml` says it is kept; none if the map was never begun.
    pub fn read(
        dir: &Path,
        find: fn(Keeper, &str, &str) -> anyhow::Result<String>,
    ) -> anyhow::Result<Option<Index>> {
        let Some(card) = key::card(dir)? else {
            return Ok(None);
        };
        Ok(Some(Index::read_only(dir, &key::identity(&card, find)?)))
    }

    /// The index in `dir`, written to with `recipient` alone: what is new
    /// is written down, and nothing is read.
    #[cfg(test)]
    pub fn writer(dir: &Path, recipient: age::x25519::Recipient) -> Index {
        let (cache, _) = Journal::open(dir, "cache", recipient.clone(), None);
        let (kept, _) = Journal::open(dir, "keep", recipient.clone(), None);
        let (taught, _) = Journal::open(dir, "taught", recipient, None);
        Index {
            cache: Some(cache),
            kept: Some(kept),
            taught: Some(taught),
            ..Index::memory()
        }
    }

    /// The index in `dir`, its key found where `map.toml` says it is kept;
    /// or, the first time, a key made and kept in the first of `keepers`.
    /// Why the map is off, if it cannot be opened.
    pub fn open(
        dir: &Path,
        keepers: fn() -> Vec<Keeper>,
        find: fn(Keeper, &str, &str) -> anyhow::Result<String>,
        keep: crate::setup::Keep,
    ) -> Result<Index, String> {
        let said = |e: anyhow::Error| format!("{e:#}");
        let identity = match key::card(dir).map_err(said)? {
            Some(card) => key::identity(&card, find).map_err(said)?,
            None => {
                let keeper = keepers().first().copied().unwrap_or(Keeper::Vault);
                if keeper == Keeper::Vault && !crate::vault::is_open() {
                    return Err(
                        "the map's key would be kept in the vault, which is locked".to_string()
                    );
                }
                key::make(dir, keeper, keep).map_err(said)?.1
            }
        };
        Ok(Index::open_with(dir, &identity))
    }

    /// The index in `dir`, opened as `open` opens it, if its key was ever
    /// made; none if not, for the map is begun only once it is wanted.
    pub fn open_made(
        dir: &Path,
        keepers: fn() -> Vec<Keeper>,
        find: fn(Keeper, &str, &str) -> anyhow::Result<String>,
        keep: crate::setup::Keep,
    ) -> Result<Option<Index>, String> {
        match key::card(dir) {
            Ok(None) => Ok(None),
            _ => Index::open(dir, keepers, find, keep).map(Some),
        }
    }

    /// Write the journals again as a base each, where their logs run long
    /// or the cache could not be read.
    fn compact(&mut self) {
        let snapshot = self.cache_events();
        if let Some(cache) = &mut self.cache
            && (cache.fault().is_some() || cache.logs() > journal::LOGS)
            && let Err(e) = cache.compact(&snapshot)
        {
            self.fault = Some(format!("{e:#}"));
        }
        let snapshot = self.keep.events();
        if let Some(kept) = &mut self.kept
            && kept.fault().is_none()
            && kept.logs() > journal::LOGS
            && let Err(e) = kept.compact(&snapshot)
        {
            self.fault = Some(format!("{e:#}"));
        }
        if let Some(taught) = &mut self.taught
            && taught.fault().is_none()
            && taught.logs() > journal::LOGS
            && let Err(e) = taught.compact(&self.filter.taught)
        {
            self.fault = Some(format!("{e:#}"));
        }
    }

    /// Learn from what was done with the message `key`: thrown away, or
    /// kept; and write it down. Nothing is learnt of a message not indexed,
    /// or while the journal could not be read.
    pub fn teach(&mut self, key: &Key, thrown: bool) -> anyhow::Result<()> {
        if self.taught.as_ref().is_some_and(|t| t.fault().is_some()) {
            return Ok(());
        }
        let Some(r) = self.records.get(key) else {
            return Ok(());
        };
        let t = crate::filter::Taught {
            key: key.clone(),
            thrown,
            tokens: crate::filter::Filter::tokens(r, &self.known()),
            message_id: r.message_id.clone(),
            seeded: false,
            ..Default::default()
        };
        self.learn(t)
    }

    fn learn(&mut self, mut t: crate::filter::Taught) -> anyhow::Result<()> {
        if t.at == 0 {
            t.at = jiff::Timestamp::now().as_second();
        }
        self.filter.learn(t.clone());
        self.version += 1;
        if let Some(taught) = &mut self.taught {
            taught.push(t)?;
        }
        Ok(())
    }

    /// What is known beside each message: whom you wrote to, read from the
    /// mail you sent.
    pub fn known(&self) -> crate::filter::Known {
        let written = self
            .records
            .values()
            .filter(|r| self.system(&r.key).sent)
            .flat_map(|r| r.to.iter().chain(&r.cc))
            .map(|a| a.to_lowercase())
            .collect();
        crate::filter::Known { written }
    }

    /// A message of `account` put back from the Trash, by its `Message-ID`:
    /// learnt as kept, as it was read when it was thrown away, since the
    /// index has let it go. Whether it was known.
    pub fn put_back(&mut self, account: &str, message_id: &str) -> anyhow::Result<bool> {
        if message_id.is_empty() || self.taught.as_ref().is_some_and(|t| t.fault().is_some()) {
            return Ok(false);
        }
        let Some(thrown) = self
            .filter
            .taught
            .iter()
            .rev()
            .find(|t| t.key.account == account && t.message_id == message_id)
            .filter(|t| t.thrown)
            .cloned()
        else {
            return Ok(false);
        };
        self.learn(crate::filter::Taught {
            thrown: false,
            seeded: false,
            auto: false,
            at: 0,
            ..thrown
        })?;
        Ok(true)
    }

    /// Throw away the message `key` on its own, sure enough: written down,
    /// with who sent it, what about, and how sure it was, never learnt.
    pub fn throw_on_its_own(&mut self, key: &Key) -> anyhow::Result<()> {
        if self.taught.as_ref().is_some_and(|t| t.fault().is_some()) {
            anyhow::bail!("what was learnt cannot be read");
        }
        let Some(r) = self.records.get(key) else {
            return Ok(());
        };
        let tokens = crate::filter::Filter::tokens(r, &self.known());
        let t = crate::filter::Taught {
            key: key.clone(),
            thrown: true,
            odds: self.filter.odds(&tokens),
            tokens,
            message_id: r.message_id.clone(),
            auto: true,
            from: r.from.clone(),
            subject: r.subject.clone(),
            ..Default::default()
        };
        self.learn(t)
    }

    /// What happened after `since`: what was thrown away on its own, each
    /// put back since or not, and how many of your own decisions were
    /// learnt.
    pub fn debrief(&self, since: i64) -> crate::filter::Debrief {
        let mut d = crate::filter::Debrief::default();
        let taught = &self.filter.taught;
        for (i, t) in taught.iter().enumerate().filter(|(_, t)| t.at > since) {
            if t.auto {
                let back = taught[i + 1..].iter().any(|u| {
                    !u.auto
                        && !u.thrown
                        && u.key.account == t.key.account
                        && u.message_id == t.message_id
                });
                d.thrown.push(crate::filter::Thrown {
                    key: t.key.clone(),
                    from: t.from.clone(),
                    subject: t.subject.clone(),
                    odds: t.odds,
                    why: self.filter.why(&t.tokens),
                    message_id: t.message_id.clone(),
                    at: t.at,
                    back,
                });
            } else if !t.seeded {
                d.decided += 1;
                if t.thrown {
                    d.thrown_by_you += 1;
                } else {
                    d.kept += 1;
                }
            }
        }
        d
    }

    /// Learn from `seeds`, read from where mail already is, each not yet
    /// learnt; how many were.
    pub fn seed(&mut self, seeds: Vec<crate::filter::Taught>) -> anyhow::Result<usize> {
        if self.taught.as_ref().is_some_and(|t| t.fault().is_some()) {
            anyhow::bail!("what was learnt cannot be read, so nothing more is learnt");
        }
        let known: std::collections::HashSet<Key> =
            self.filter.taught.iter().map(|t| t.key.clone()).collect();
        let mut n = 0;
        for t in seeds {
            if !known.contains(&t.key) {
                self.learn(t)?;
                n += 1;
            }
        }
        Ok(n)
    }

    /// Forget what was read from the Trash and your labels: what is learnt
    /// is learnt again from your own decisions alone, and written so. How
    /// many were forgotten.
    pub fn unseed(&mut self) -> anyhow::Result<usize> {
        if self.taught.as_ref().is_some_and(|t| t.fault().is_some()) {
            anyhow::bail!("what was learnt cannot be read, so it is left as it is");
        }
        let all = std::mem::take(&mut self.filter.taught);
        let n = all.iter().filter(|t| t.seeded).count();
        self.filter = crate::filter::Filter::default();
        for t in all.into_iter().filter(|t| !t.seeded) {
            self.filter.learn(t);
        }
        self.version += 1;
        if let Some(taught) = &mut self.taught {
            taught.compact(&self.filter.taught)?;
        }
        Ok(n)
    }

    /// The mail you kept, filed under a label and out of the Inbox, as
    /// seeds: the newest `limit`, each with when it was sent.
    pub fn kept_seeds(&self, limit: usize) -> Vec<(i64, crate::filter::Taught)> {
        let known = self.known();
        let mut kept: Vec<(i64, crate::filter::Taught)> = self
            .records
            .values()
            .filter(|r| {
                let s = self.system(&r.key);
                !s.inbox && !s.sent && !s.draft && !self.labels(&r.key).is_empty()
            })
            .map(|r| {
                (
                    r.date.unwrap_or(r.first_seen),
                    crate::filter::Taught {
                        key: r.key.clone(),
                        thrown: false,
                        tokens: crate::filter::Filter::tokens(r, &known),
                        message_id: r.message_id.clone(),
                        seeded: true,
                        ..Default::default()
                    },
                )
            })
            .collect();
        kept.sort_by_key(|(at, t)| (std::cmp::Reverse(*at), t.key.clone()));
        kept.truncate(limit);
        kept
    }

    /// The message of `account` at `uid` in `folder`, by its key.
    pub fn key_at(&self, account: &str, folder: &str, uid: u32) -> Option<Key> {
        self.records
            .values()
            .find(|r| {
                r.key.account == account
                    && r.places.iter().any(|p| p.folder == folder && p.uid == uid)
            })
            .map(|r| r.key.clone())
    }

    /// The cache, as events to write a base of.
    fn cache_events(&self) -> Vec<Cached> {
        let folders = self.folders.iter().map(|((a, f), s)| Cached::Folder {
            account: a.clone(),
            folder: f.clone(),
            state: s.clone(),
        });
        let records = self
            .records
            .values()
            .map(|r| Cached::Upsert(Box::new(r.clone())));
        folders.chain(records).collect()
    }

    /// Let go of what the servers said, to be read from them again: the
    /// cache written again as a base of nothing, `keep` left as it is.
    pub fn forget_cache(&mut self) -> anyhow::Result<()> {
        self.records.clear();
        self.folders.clear();
        self.version += 1;
        if let Some(cache) = &mut self.cache {
            cache.compact(&[])?;
        }
        Ok(())
    }

    /// Why nothing is written to `keep`, if it could not be read.
    pub fn keep_fault(&self) -> Option<&str> {
        self.kept.as_ref().and_then(|k| k.fault())
    }

    /// Take in what the servers say, and write it down.
    pub fn cache(&mut self, events: Vec<Cached>) -> anyhow::Result<()> {
        let mut written = Ok(());
        for e in events {
            if self.take(&e)
                && let Some(cache) = &mut self.cache
                && written.is_ok()
            {
                written = cache.push(e);
            }
        }
        written
    }

    /// Take in what only letterbox knows, and write it down; nothing is
    /// taken in when it cannot be written.
    pub fn keep(&mut self, events: Vec<Kept>) -> anyhow::Result<()> {
        if let Some(why) = self.keep_fault() {
            anyhow::bail!("{why}");
        }
        for e in events {
            self.keep.apply(&e);
            self.version += 1;
            if let Some(kept) = &mut self.kept {
                kept.push(e)?;
            }
        }
        Ok(())
    }

    /// Write down what is held, as on quitting.
    pub fn flush(&mut self) -> anyhow::Result<()> {
        if let Some(cache) = &mut self.cache {
            cache.flush()?;
        }
        if let Some(kept) = &mut self.kept
            && kept.fault().is_none()
        {
            kept.flush()?;
        }
        if let Some(taught) = &mut self.taught
            && taught.fault().is_none()
        {
            taught.flush()?;
        }
        Ok(())
    }

    /// Write down what has been held a while.
    pub fn tick(&mut self) -> anyhow::Result<()> {
        if let Some(cache) = &mut self.cache {
            cache.tick()?;
        }
        if let Some(kept) = &mut self.kept
            && kept.fault().is_none()
        {
            kept.tick()?;
        }
        if let Some(taught) = &mut self.taught
            && taught.fault().is_none()
        {
            taught.tick()?;
        }
        Ok(())
    }

    /// Take in one of what the servers say; whether it changed anything.
    fn take(&mut self, e: &Cached) -> bool {
        let changed = match e {
            Cached::Upsert(r) => match self.records.get_mut(&r.key) {
                Some(old) => {
                    let mut new = (**r).clone();
                    new.first_seen = old.first_seen;
                    for p in &old.places {
                        if !new.places.iter().any(|q| q.folder == p.folder) {
                            new.places.push(p.clone());
                        }
                    }
                    let changed = *old != new;
                    *old = new;
                    changed
                }
                None => {
                    self.records.insert(r.key.clone(), (**r).clone());
                    true
                }
            },
            Cached::Place { key, place } => match self.records.get_mut(key) {
                Some(r) => {
                    let before = r.places.clone();
                    r.places.retain(|p| p.folder != place.folder);
                    r.places.push(place.clone());
                    r.places != before
                }
                None => false,
            },
            Cached::Unplace { key, folder, uid } => match self.records.get_mut(key) {
                Some(r) => {
                    let n = r.places.len();
                    r.places.retain(|p| !(p.folder == *folder && p.uid == *uid));
                    r.places.len() != n
                }
                None => false,
            },
            Cached::Flags { key, flags } => match self.records.get_mut(key) {
                Some(r) if r.flags != *flags => {
                    r.flags = *flags;
                    true
                }
                _ => false,
            },
            Cached::Labels { key, labels } => match self.records.get_mut(key) {
                Some(r) if r.server != *labels => {
                    r.server = labels.clone();
                    true
                }
                _ => false,
            },
            Cached::Folder {
                account,
                folder,
                state,
            } => {
                let at = (account.clone(), folder.clone());
                let changed = self.folders.get(&at) != Some(state);
                self.folders.insert(at, state.clone());
                changed
            }
            Cached::Reset { account, folder } => {
                let mut changed = self
                    .folders
                    .remove(&(account.clone(), folder.clone()))
                    .is_some();
                for r in self
                    .records
                    .values_mut()
                    .filter(|r| r.key.account == *account)
                {
                    let n = r.places.len();
                    r.places.retain(|p| p.folder != *folder);
                    changed |= r.places.len() != n;
                }
                changed
            }
            Cached::Forget { key } => self.records.remove(key).is_some(),
        };
        if changed {
            self.version += 1;
        }
        changed
    }
}
