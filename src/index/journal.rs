//! A journal: what happened, as events, in files beside the vault that
//! only the map's key opens. It is a base, all that was known when it was
//! written, and the logs after it, each a whole age stream of JSON lines
//! encrypted to the key's public half: written without the secret, read
//! only with it. Each file is written beside its name and then renamed to
//! it, so it is there whole or not at all, and a base is written before
//! the files it takes the place of are removed.

use age::x25519::{Identity, Recipient};
use anyhow::Context;
use serde::Serialize;
use serde::de::DeserializeOwned;
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

/// How many events are held before they are written, and for how long.
const EVENTS: usize = 500;
const WAIT: Duration = Duration::from_secs(5);
/// How many logs follow a base before it is written again, at start-up.
pub const LOGS: usize = 64;

pub struct Journal<T: Serialize> {
    dir: PathBuf,
    name: &'static str,
    recipient: Recipient,
    /// The base the logs follow, by its number.
    generation: u64,
    /// The next log's number, and how many logs follow the base.
    next: u64,
    logs: usize,
    unwritten: Vec<T>,
    since: Instant,
    /// Why a file of it could not be read. Nothing is written while this
    /// is set: a bad read must not be followed by what assumes it.
    fault: Option<String>,
}

/// A file of a journal, by its name: its generation, and its number among
/// the logs if it is one, else it is the base.
fn parse(name: &str, file: &str) -> Option<(u64, Option<u64>)> {
    let rest = file.strip_prefix(name)?.strip_prefix('.')?;
    if let Some(g) = rest.strip_suffix(".base.age") {
        return Some((g.parse().ok()?, None));
    }
    let (g, s) = rest.strip_suffix(".log.age")?.split_once('.')?;
    Some((g.parse().ok()?, Some(s.parse().ok()?)))
}

fn base_name(name: &str, generation: u64) -> String {
    format!("{name}.{generation:08}.base.age")
}

fn log_name(name: &str, generation: u64, n: u64) -> String {
    format!("{name}.{generation:08}.{n:08}.log.age")
}

impl<T: Serialize + DeserializeOwned> Journal<T> {
    /// The journal `name` in `dir`, and the events it holds, read with
    /// `identity`; without one, it is only written to, and nothing is read.
    pub fn open(
        dir: &Path,
        name: &'static str,
        recipient: Recipient,
        identity: Option<&Identity>,
    ) -> (Self, Vec<T>) {
        let mut journal = Journal {
            dir: dir.to_path_buf(),
            name,
            recipient,
            generation: 0,
            next: 0,
            logs: 0,
            unwritten: Vec::new(),
            since: Instant::now(),
            fault: None,
        };
        let files = match journal.files() {
            Ok(files) => files,
            Err(e) => {
                journal.fault = Some(format!("cannot read {}: {e}", dir.display()));
                return (journal, Vec::new());
            }
        };
        let newest_base = files.iter().filter(|f| f.1.is_none()).map(|f| f.0).max();
        let generation = newest_base
            .or_else(|| files.iter().map(|f| f.0).max())
            .unwrap_or(0);
        let mut logs: Vec<(u64, PathBuf)> = files
            .iter()
            .filter(|f| f.0 == generation)
            .filter_map(|f| Some((f.1?, f.2.clone())))
            .collect();
        logs.sort();
        journal.generation = generation;
        journal.next = logs.last().map_or(0, |l| l.0 + 1);
        journal.logs = logs.len();
        let Some(identity) = identity else {
            return (journal, Vec::new());
        };
        let mut events = Vec::new();
        let base = newest_base.map(|g| dir.join(base_name(name, g)));
        for path in base.into_iter().chain(logs.into_iter().map(|l| l.1)) {
            match read(&path, identity) {
                Ok(mut more) => events.append(&mut more),
                Err(e) => {
                    journal.fault = Some(format!("{e:#}"));
                    break;
                }
            }
        }
        (journal, events)
    }
}

impl<T: Serialize> Journal<T> {
    /// Its files: each one's generation, its number if it is a log, and
    /// where it is.
    fn files(&self) -> std::io::Result<Vec<(u64, Option<u64>, PathBuf)>> {
        let entries = match std::fs::read_dir(&self.dir) {
            Ok(entries) => entries,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(Vec::new()),
            Err(e) => return Err(e),
        };
        let mut out = Vec::new();
        for entry in entries {
            let entry = entry?;
            let file = entry.file_name();
            if let Some((g, n)) = parse(self.name, &file.to_string_lossy()) {
                out.push((g, n, entry.path()));
            }
        }
        Ok(out)
    }

    /// Why it could not be read, if it could not.
    pub fn fault(&self) -> Option<&str> {
        self.fault.as_deref()
    }

    /// How many logs follow its base.
    pub fn logs(&self) -> usize {
        self.logs
    }

    /// Hold `event` to be written, and write what is held once there is
    /// enough of it.
    pub fn push(&mut self, event: T) -> anyhow::Result<()> {
        self.writable()?;
        self.unwritten.push(event);
        if self.unwritten.len() >= EVENTS {
            self.flush()?;
        }
        Ok(())
    }

    /// Write what is held, if it has been held a while.
    pub fn tick(&mut self) -> anyhow::Result<()> {
        if !self.unwritten.is_empty() && self.since.elapsed() >= WAIT {
            self.flush()?;
        }
        Ok(())
    }

    /// Write what is held, as a log of its own.
    pub fn flush(&mut self) -> anyhow::Result<()> {
        if self.unwritten.is_empty() {
            return Ok(());
        }
        self.writable()?;
        let path = self
            .dir
            .join(log_name(self.name, self.generation, self.next));
        self.write(&path, &self.unwritten)?;
        self.unwritten.clear();
        self.next += 1;
        self.logs += 1;
        self.since = Instant::now();
        Ok(())
    }

    /// Write `all`, everything the journal now holds, as a new base, and
    /// then remove the files it takes the place of; what was held unwritten
    /// is in `all`. Written over what could not be read: for a journal
    /// that can be made again from the servers, as the cache.
    pub fn compact(&mut self, all: &[T]) -> anyhow::Result<()> {
        let generation = self.generation + 1;
        let path = self.dir.join(base_name(self.name, generation));
        self.write(&path, all)?;
        let old: Vec<PathBuf> = self
            .files()?
            .into_iter()
            .filter(|f| f.0 < generation)
            .map(|f| f.2)
            .collect();
        self.generation = generation;
        self.next = 0;
        self.logs = 0;
        self.unwritten.clear();
        self.fault = None;
        for path in old {
            std::fs::remove_file(&path)
                .with_context(|| format!("cannot remove {}", path.display()))?;
        }
        Ok(())
    }

    fn writable(&self) -> anyhow::Result<()> {
        if let Some(why) = &self.fault {
            anyhow::bail!("{why}");
        }
        Ok(())
    }

    /// Write `events` to `path`, encrypted, beside it first.
    fn write(&self, path: &Path, events: &[T]) -> anyhow::Result<()> {
        let mut plain = Vec::new();
        for e in events {
            serde_json::to_writer(&mut plain, e)?;
            plain.push(b'\n');
        }
        let sealed = age::encrypt(&self.recipient, &plain).context("cannot encrypt the map")?;
        create_private(&self.dir)?;
        let tmp = path.with_extension("age.new");
        crate::vault::write_private(&tmp, &sealed)?;
        std::fs::rename(&tmp, path).with_context(|| format!("cannot write {}", path.display()))
    }
}

impl<T: Serialize> Drop for Journal<T> {
    fn drop(&mut self) {
        // What is held is written, if it can be: a journal is flushed on
        // quitting, and this is for a journal let go otherwise.
        let _ = self.flush();
    }
}

/// The events of the file at `path`, decrypted with `identity`.
fn read<T: DeserializeOwned>(path: &Path, identity: &Identity) -> anyhow::Result<Vec<T>> {
    let bytes = std::fs::read(path).with_context(|| format!("cannot read {}", path.display()))?;
    let plain = age::decrypt(identity, &bytes)
        .map_err(|e| anyhow::anyhow!("cannot read {}: {e}", path.display()))?;
    let mut events = Vec::new();
    for (i, line) in plain.split(|&b| b == b'\n').enumerate() {
        if line.is_empty() {
            continue;
        }
        let event = serde_json::from_slice(line)
            .with_context(|| format!("cannot read {}, line {}", path.display(), i + 1))?;
        events.push(event);
    }
    Ok(events)
}

/// `dir`, made if it is not there, only you may look in, where it is made.
pub fn create_private(dir: &Path) -> std::io::Result<()> {
    let mut builder = std::fs::DirBuilder::new();
    builder.recursive(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::DirBuilderExt;
        builder.mode(0o700);
    }
    builder.create(dir)
}
