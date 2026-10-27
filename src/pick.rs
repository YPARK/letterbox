//! Files picked to be sent with a message, one folder at a time, as senna
//! browses them: `..`, the folder's folders, then its files with their
//! sizes, the newest starred and begun on. Typing narrows to the names
//! that hold what is typed; typed from a `/`, it is a path, followed as
//! soon as its folder is there, and into a folder its name alone begins.
//! Space marks a file, `C-a` all those shown, and enter takes the marked,
//! or the one picked. A folder to save in is picked so too, its folders
//! alone shown, and `.` under `..` to save in the one it is in, or in a
//! new one named by what is typed, when no folder there is so named.

use ratatui::crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
use std::collections::BTreeSet;
use std::path::{Path, PathBuf};
use std::time::SystemTime;

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Entry {
    Up,
    /// The folder shown itself, to save in.
    Here,
    Dir(String),
    File {
        name: String,
        size: u64,
    },
}

impl Entry {
    pub fn name(&self) -> &str {
        match self {
            Entry::Up => "..",
            Entry::Here => ".",
            Entry::Dir(n) | Entry::File { name: n, .. } => n,
        }
    }
}

/// What a key did in the picker.
#[derive(Debug, PartialEq, Eq)]
pub enum Picked {
    Stay,
    Cancel,
    /// The files taken, never none.
    Take(Vec<PathBuf>),
    /// The folder to save in, perhaps not there yet.
    Folder(PathBuf),
}

pub struct Picker {
    pub dir: PathBuf,
    entries: Vec<Entry>,
    /// What is typed: letters narrow the list to names holding them; from
    /// a `/`, a path the picker follows.
    pub filter: String,
    /// The entry picked, among those shown.
    pub row: usize,
    /// Files marked, in any folder.
    pub marked: BTreeSet<PathBuf>,
    /// The folder's newest file, starred.
    pub newest: Option<String>,
    /// Why the folder could not be read, if it could not.
    pub note: Option<String>,
    /// Picking a folder, not files: its folders alone shown.
    pub folders: bool,
}

impl Picker {
    /// The picker in `dir`, on its newest file.
    pub fn open(dir: PathBuf) -> Picker {
        Picker::new(dir, false)
    }

    /// The picker of a folder in `dir`, on `dir` itself.
    pub fn folder(dir: PathBuf) -> Picker {
        Picker::new(dir, true)
    }

    fn new(dir: PathBuf, folders: bool) -> Picker {
        let mut p = Picker {
            dir,
            entries: Vec::new(),
            filter: String::new(),
            row: 0,
            marked: BTreeSet::new(),
            newest: None,
            note: None,
            folders,
        };
        p.read(None);
        p
    }

    /// Read the folder anew, the filter cleared; the entry `select` picked,
    /// else the newest file, else the first after `..`.
    fn read(&mut self, select: Option<&str>) {
        self.filter.clear();
        self.load();
        let want = select.map(str::to_string).or_else(|| self.newest.clone());
        let shown = self.shown();
        let at = want.and_then(|w| shown.iter().position(|e| e.name() == w));
        self.row = at.unwrap_or_else(|| self.first(&shown));
    }

    /// The entry picked when no other is, of those `shown`: the first
    /// folder narrowed to when picking a folder, else saving here; the
    /// first after `..`.
    fn first(&self, shown: &[&Entry]) -> usize {
        let narrowed = !self.narrowing().is_empty();
        match shown.iter().position(|e| matches!(e, Entry::Dir(_))) {
            Some(at) if self.folders && narrowed => at,
            _ => usize::from(shown.len() > 1),
        }
    }

    /// What the entries are narrowed by: what is typed, or a typed path's
    /// name begun after its last `/`; none once that name is a folder
    /// gone into.
    fn narrowing(&self) -> &str {
        match self.typed_path() {
            Some(_) if self.completed() => "",
            Some((_, name)) => name,
            None => &self.filter,
        }
    }

    /// The folder `.` saves in: the one shown; or, when what is typed
    /// names none of its folders, a new one so named, in the folder a
    /// typed path names.
    pub fn here(&self) -> PathBuf {
        let name = self.narrowing();
        let named = self.shown().iter().any(|e| matches!(e, Entry::Dir(_)));
        if named || matches!(name, "" | "." | "..") {
            return self.dir.clone();
        }
        match self.typed_path() {
            Some((folder, _)) => Path::new(folder)
                .components()
                .collect::<PathBuf>()
                .join(name),
            None => self.dir.join(name),
        }
    }

    /// List the folder: its folders, then its files, each by name; the
    /// newest file found.
    fn load(&mut self) {
        let mut dirs = Vec::new();
        let mut files = Vec::new();
        let mut newest: Option<(SystemTime, String)> = None;
        self.note = None;
        match std::fs::read_dir(&self.dir) {
            Ok(read) => {
                for e in read.flatten() {
                    let name = e.file_name().to_string_lossy().into_owned();
                    // Through links, as a folder or a file is reached.
                    let Ok(meta) = std::fs::metadata(e.path()) else {
                        continue;
                    };
                    if meta.is_dir() {
                        dirs.push(name);
                    } else if !self.folders {
                        // As new as another: the first by name, whatever
                        // order the folder is read in.
                        if !name.starts_with('.')
                            && let Ok(at) = meta.modified()
                            && newest
                                .as_ref()
                                .is_none_or(|(t, n)| at > *t || (at == *t && name < *n))
                        {
                            newest = Some((at, name.clone()));
                        }
                        files.push((name, meta.len()));
                    }
                }
            }
            Err(e) => self.note = Some(format!("cannot read {}: {e}", self.dir.display())),
        }
        let by = |a: &str, b: &str| a.to_lowercase().cmp(&b.to_lowercase()).then(a.cmp(b));
        dirs.sort_by(|a, b| by(a, b));
        files.sort_by(|a, b| by(&a.0, &b.0));
        self.newest = newest.map(|(_, n)| n);
        let here = self.folders.then_some(Entry::Here);
        self.entries = std::iter::once(Entry::Up)
            .chain(here)
            .chain(dirs.into_iter().map(Entry::Dir))
            .chain(
                files
                    .into_iter()
                    .map(|(name, size)| Entry::File { name, size }),
            )
            .collect();
    }

    /// The typed path's folder, up to its last `/`, and the name begun
    /// after it, when what is typed is a path.
    fn typed_path(&self) -> Option<(&str, &str)> {
        if !self.filter.starts_with('/') {
            return None;
        }
        let at = self.filter.rfind('/')? + 1;
        Some((&self.filter[..at], &self.filter[at..]))
    }

    /// Follow a typed path: into its folder as soon as that folder is
    /// there, and into the folder its name begins once no other entry
    /// there begins so.
    fn follow(&mut self) {
        let Some((folder, name)) = self.typed_path() else {
            return;
        };
        let folder: PathBuf = Path::new(folder).components().collect();
        if !folder.is_dir() {
            return;
        }
        let into = folder.join(name);
        let only = || {
            std::fs::read_dir(&folder).map_or(0, |d| {
                d.flatten()
                    .filter(|e| e.file_name().to_string_lossy().starts_with(name))
                    .count()
            }) == 1
        };
        let target = if !name.is_empty() && into.is_dir() && only() {
            into
        } else {
            folder
        };
        if target != self.dir {
            self.dir = target;
            self.load();
        }
    }

    /// Whether the typed path's name was taken as far as the folder it
    /// begins, which is then the one shown.
    fn completed(&self) -> bool {
        self.typed_path().is_some_and(|(folder, name)| {
            !name.is_empty()
                && Path::new(folder)
                    .join(name)
                    .components()
                    .collect::<PathBuf>()
                    == self.dir
        })
    }

    /// The entries what is typed lets through, `..` always first, and `.`
    /// when picking a folder; those
    /// whose names begin with `.` only when what is typed does.
    pub fn shown(&self) -> Vec<&Entry> {
        let f = self.narrowing().to_lowercase();
        let hidden = f.starts_with('.');
        self.entries
            .iter()
            .filter(|e| {
                matches!(e, Entry::Up | Entry::Here)
                    || ((hidden || !e.name().starts_with('.'))
                        && e.name().to_lowercase().contains(&f))
            })
            .collect()
    }

    /// The entry picked.
    pub fn current(&self) -> Option<&Entry> {
        self.shown().get(self.row).copied()
    }

    fn up(&mut self) {
        let from = self
            .dir
            .file_name()
            .map(|n| n.to_string_lossy().into_owned());
        if let Some(parent) = self.dir.parent().map(Path::to_path_buf) {
            self.dir = parent;
            self.read(from.as_deref());
        }
    }

    /// Into the folder picked, or up for `..`.
    fn open_here(&mut self) {
        match self.current().cloned() {
            Some(Entry::Up) => self.up(),
            Some(Entry::Dir(name)) => {
                self.dir.push(name);
                self.read(None);
            }
            _ => {}
        }
    }

    /// Mark the file picked, or let it go.
    fn mark_here(&mut self) {
        if let Some(Entry::File { name, .. }) = self.current() {
            let path = self.dir.join(name);
            if !self.marked.remove(&path) {
                self.marked.insert(path);
            }
        }
    }

    /// The files taken: those marked, else the one picked.
    fn take(&mut self) -> Picked {
        if !self.marked.is_empty() {
            return Picked::Take(std::mem::take(&mut self.marked).into_iter().collect());
        }
        match self.current() {
            Some(Entry::File { name, .. }) => Picked::Take(vec![self.dir.join(name)]),
            _ => Picked::Stay,
        }
    }

    pub fn step(&mut self, by: isize) {
        let last = self.shown().len().saturating_sub(1);
        self.row = self.row.saturating_add_signed(by).min(last);
    }

    /// The entry `i` of those shown, clicked: a folder opened, a file
    /// marked or let go, or the folder shown taken.
    pub fn click(&mut self, i: usize) -> Picked {
        self.row = i.min(self.shown().len().saturating_sub(1));
        match self.current() {
            Some(Entry::Here) => return Picked::Folder(self.here()),
            Some(Entry::File { .. }) => self.mark_here(),
            _ => self.open_here(),
        }
        Picked::Stay
    }

    pub fn key(&mut self, key: KeyEvent) -> Picked {
        let ctrl = key.modifiers.contains(KeyModifiers::CONTROL);
        let alt = key.modifiers.contains(KeyModifiers::ALT);
        match key.code {
            KeyCode::Esc => return Picked::Cancel,
            KeyCode::Char('g') if ctrl => return Picked::Cancel,
            KeyCode::Up => self.step(-1),
            KeyCode::Down => self.step(1),
            KeyCode::Char('p') if ctrl => self.step(-1),
            KeyCode::Char('n') if ctrl => self.step(1),
            KeyCode::PageUp => self.step(-10),
            KeyCode::PageDown => self.step(10),
            KeyCode::Home => self.row = 0,
            KeyCode::End => self.step(isize::MAX),
            KeyCode::Left => self.up(),
            KeyCode::Right => self.open_here(),
            KeyCode::Backspace => {
                if self.filter.pop().is_none() {
                    self.up();
                } else {
                    self.follow();
                    self.row = self.first(&self.shown());
                }
            }
            KeyCode::Enter => match self.current() {
                Some(Entry::File { .. }) => return self.take(),
                Some(Entry::Here) => return Picked::Folder(self.here()),
                _ => self.open_here(),
            },
            KeyCode::Char(' ') => {
                self.mark_here();
                self.step(1);
            }
            KeyCode::Char('a') if ctrl => {
                let dir = self.dir.clone();
                let files: Vec<PathBuf> = self
                    .shown()
                    .into_iter()
                    .filter_map(|e| match e {
                        Entry::File { name, .. } => Some(dir.join(name)),
                        _ => None,
                    })
                    .collect();
                self.marked.extend(files);
            }
            KeyCode::Char('~') if self.filter.is_empty() => {
                if let Some(home) = std::env::var_os("HOME") {
                    self.dir = PathBuf::from(home);
                    self.read(None);
                }
            }
            // Typing narrows the list, the picked entry kept where it is
            // still shown.
            KeyCode::Char(c) if !ctrl && !alt => {
                let on = self
                    .current()
                    .filter(|e| matches!(e, Entry::Dir(_) | Entry::File { .. }))
                    .map(|e| e.name().to_string());
                self.filter.push(c);
                self.follow();
                let shown = self.shown();
                let at = |name: &str| shown.iter().position(|e| e.name() == name);
                let kept = on
                    .as_deref()
                    .and_then(at)
                    .or_else(|| self.newest.as_deref().and_then(at));
                self.row = kept.unwrap_or_else(|| self.first(&shown));
            }
            _ => {}
        }
        Picked::Stay
    }
}
