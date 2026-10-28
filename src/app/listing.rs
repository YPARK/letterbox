//! The folders and the list: what is listed of a folder, a label or a
//! cluster, page by page, searched and narrowed, and the message picked.
use super::*;

/// The roles whose folders are listed together, every account's, in the
/// sidebar's order.
pub(super) const TOGETHER: [Role; 3] = [Role::Inbox, Role::Sent, Role::Drafts];

/// What the list shows: a folder of one account's, or every account's
/// folder of a role.
#[derive(Clone, PartialEq, Debug)]
pub enum View {
    Folder(usize, String),
    All(Role),
    /// The map's clusters, every one, or one; or what fits nowhere.
    Map,
    Cluster(u32),
    Loners,
    /// What holds every word searched for, of every folder of every
    /// account, as letterbox knows it; or of one cluster, or of what fits
    /// nowhere, if it was listed when searched.
    Found(String, Within),
}

/// What a search was of: everything, or the messages a cluster held, or
/// what fitted nowhere, when it was searched. Held by its messages, not
/// the cluster's number: the map drawn again numbers its clusters anew.
#[derive(Clone, PartialEq, Debug)]
pub enum Within {
    Everywhere,
    Of {
        name: String,
        keys: Arc<std::collections::BTreeSet<crate::index::Key>>,
    },
}

/// A folder's messages, as many as are listed, newest first as its server
/// numbers them; and how many it holds.
pub(crate) struct Part {
    pub account: usize,
    pub folder: String,
    pub messages: Vec<Summary>,
    pub total: u32,
    pub loading: bool,
    pub error: Option<String>,
}

impl Part {
    pub(super) fn is(&self, account: usize, folder: &str) -> bool {
        self.account == account && self.folder == folder
    }

    /// Whether it holds more than is listed.
    pub(super) fn partial(&self) -> bool {
        self.messages.len() < self.total as usize
    }

    /// The date of the oldest dated message listed.
    pub(super) fn oldest(&self) -> Option<i64> {
        self.messages.iter().filter_map(|m| m.date).min()
    }
}

/// How old a message of folders together may be and still be listed: as
/// old as the newest of the oldest listed of the folders listed in part,
/// as what is still to come of them would come before anything older.
pub(super) fn cut(parts: &[Part]) -> Option<i64> {
    parts
        .iter()
        .filter(|p| p.partial())
        .filter_map(Part::oldest)
        .max()
}

/// What is listed: the folders the view shows, each as far as it is.
pub struct Listing {
    pub view: View,
    pub(crate) parts: Vec<Part>,
    /// What the map lists, as its snapshot has it, in its order: not asked
    /// of the servers, which keep no clusters.
    pub(crate) fixed: Option<Vec<Listed>>,
}

impl Listing {
    pub(super) fn part_mut(&mut self, account: usize, folder: &str) -> Option<&mut Part> {
        self.parts.iter_mut().find(|p| p.is(account, folder))
    }

    pub(super) fn has(&self, account: usize, folder: &str) -> bool {
        self.parts.iter().any(|p| p.is(account, folder))
    }

    /// Each folder listed, by its account and path.
    pub(super) fn keys(&self) -> Vec<(usize, String)> {
        self.parts
            .iter()
            .map(|p| (p.account, p.folder.clone()))
            .collect()
    }

    /// The account whose folder is listed, if it is one account's.
    pub fn account(&self) -> Option<usize> {
        match self.view {
            View::Folder(a, _) => Some(a),
            _ => None,
        }
    }

    /// Whether more than one account's messages are listed, each to be
    /// told by its account.
    pub fn mixed(&self) -> bool {
        self.parts
            .iter()
            .any(|p| p.account != self.parts[0].account)
    }

    pub fn loading(&self) -> bool {
        self.parts.iter().any(|p| p.loading)
    }

    /// Whether messages are still to come before any is listed.
    pub fn fetching(&self) -> bool {
        self.loading() && self.parts.iter().all(|p| p.messages.is_empty())
    }

    pub fn total(&self) -> u32 {
        if let Some(fixed) = &self.fixed {
            return fixed.len() as u32;
        }
        self.parts.iter().map(|p| p.total).sum()
    }

    pub fn error(&self) -> Option<&str> {
        self.parts.iter().find_map(|p| p.error.as_deref())
    }
}

/// A message listed, and the account and folder it is in.
#[derive(Clone, Debug)]
pub struct Listed {
    pub account: usize,
    pub folder: String,
    pub summary: Summary,
}

impl std::ops::Deref for Listed {
    type Target = Summary;
    fn deref(&self) -> &Summary {
        &self.summary
    }
}

impl Listed {
    pub(super) fn new(account: usize, folder: &str, summary: Summary) -> Self {
        Listed {
            account,
            folder: folder.to_string(),
            summary,
        }
    }

    pub(super) fn is(&self, account: usize, folder: &str, uid: u32) -> bool {
        self.account == account && self.folder == folder && self.uid == uid
    }
}

/// The messages of `parts`, as listed, with the folder each is in: one
/// folder's as its server numbers them; folders together newest first,
/// the undated last, and none older than the cut.
pub(crate) fn merged(parts: &[Part]) -> Vec<(&Part, &Summary)> {
    let mut all: Vec<(&Part, &Summary)> = parts
        .iter()
        .flat_map(|p| p.messages.iter().map(move |m| (p, m)))
        .collect();
    if let [_] = parts {
        return all;
    }
    let cut = cut(parts);
    all.retain(|(_, m)| cut.is_none_or(|c| m.date.is_none_or(|d| d >= c)));
    all.sort_by_key(|(_, m)| newest(m));
    all
}

/// What sorts messages of folders together newest first; the undated last.
pub(super) fn newest(m: &Summary) -> std::cmp::Reverse<i64> {
    std::cmp::Reverse(m.date.unwrap_or(i64::MIN))
}

impl App {
    pub(super) fn folder_mut(&mut self, account: usize, path: &str) -> Option<&mut Folder> {
        self.accounts[account]
            .folders
            .iter_mut()
            .find(|f| f.path == path)
    }

    /// The name of what is listed: a folder's, or a role's, of every
    /// account's.
    pub fn folder_name(&self) -> String {
        match self.listing.as_ref().map(|l| &l.view) {
            Some(View::All(role)) => format!("{}, all accounts", role.name()),
            Some(View::Folder(..)) => self.folder().map(|f| f.name.clone()).unwrap_or_default(),
            Some(View::Map) => "Map, every cluster".to_string(),
            Some(View::Cluster(id)) => self
                .map
                .snapshot
                .as_ref()
                .and_then(|s| s.group(*id))
                .map(|g| g.name())
                .unwrap_or_else(|| "a cluster".to_string()),
            Some(View::Loners) => "Fits nowhere".to_string(),
            Some(View::Found(words, Within::Everywhere)) => format!("Found: {words}"),
            Some(View::Found(words, Within::Of { name, .. })) => {
                format!("Found in {name}: {words}")
            }
            None => String::new(),
        }
    }

    /// What a search of the cluster listed, or of what fits nowhere, is
    /// of: its name, and the messages it holds now.
    fn within(&self, view: &View) -> Within {
        let Some(s) = &self.map.snapshot else {
            return Within::Everywhere;
        };
        let (name, members): (String, Vec<usize>) = match *view {
            View::Cluster(id) => (
                s.group(id)
                    .map(|g| g.name())
                    .unwrap_or_else(|| "a cluster".to_string()),
                (0..s.dots.len()).filter(|&i| s.picked_by(id, i)).collect(),
            ),
            View::Loners => ("what fits nowhere".to_string(), s.loners()),
            _ => return Within::Everywhere,
        };
        let keys = members.into_iter().map(|i| s.dots[i].key.clone());
        Within::Of {
            name,
            keys: Arc::new(keys.collect()),
        }
    }

    /// The folder listed, as the sidebar has it, if one account's is.
    pub fn folder(&self) -> Option<&Folder> {
        match &self.listing.as_ref()?.view {
            View::Folder(a, path) => self.folder_of(*a, path),
            _ => None,
        }
    }

    pub(super) fn folder_of(&self, account: usize, path: &str) -> Option<&Folder> {
        self.accounts
            .get(account)?
            .folders
            .iter()
            .find(|f| f.path == path)
    }

    pub(super) fn is_trash(&self, account: usize, path: &str) -> bool {
        self.folder_of(account, path)
            .is_some_and(|f| f.role == Role::Trash)
    }

    /// The account's folder of `role`, if it has one to list.
    pub(super) fn role_folder(&self, account: usize, role: Role) -> Option<&Folder> {
        self.accounts[account]
            .folders
            .iter()
            .find(|f| f.selectable && f.role == role)
    }

    /// The folders of `role`, one an account, of the accounts in use.
    pub(super) fn folders_of(&self, role: Role) -> Vec<(usize, String)> {
        self.live()
            .filter_map(|a| Some((a, self.role_folder(a, role)?.path.clone())))
            .collect()
    }

    /// How many messages of every account's folder of `role` are not read,
    /// as far as their folders are counted.
    pub fn unseen_of(&self, role: Role) -> Option<u32> {
        self.live()
            .filter_map(|a| self.role_folder(a, role)?.unseen)
            .reduce(|x, y| x + y)
    }

    /// Show a message as marked, or not, wherever it is listed, and count
    /// its folder's unread messages again.
    pub(super) fn marked(&mut self, account: usize, folder: &str, uid: u32, flag: Flag, on: bool) {
        let mut changed = false;
        let mut set = |s: &mut Summary| {
            if s.uid != uid {
                return;
            }
            let field = match flag {
                Flag::Seen => &mut s.flags.seen,
                Flag::Flagged => &mut s.flags.flagged,
                Flag::Answered => &mut s.flags.answered,
            };
            changed |= *field != on;
            *field = on;
        };
        if let Some(l) = self
            .listing
            .as_mut()
            .and_then(|l| l.part_mut(account, folder))
        {
            l.messages.iter_mut().for_each(&mut set);
        }
        let fixed = self.listing.as_mut().and_then(|l| l.fixed.as_mut());
        std::iter::once(&mut self.shown)
            .chain(self.found.as_mut())
            .chain(fixed)
            .flatten()
            .filter(|m| m.account == account && m.folder == folder)
            .for_each(|m| set(&mut m.summary));
        if let Some((messages, _)) = self.listed.get_mut(&(account, folder.to_string())) {
            messages.iter_mut().for_each(&mut set);
        }
        self.index_flag(account, folder, uid, flag, on);
        // Flagged, it is kept: learnt as not thrown away.
        if changed && flag == Flag::Flagged && on {
            self.teach_at(account, folder, uid, false);
        }
        if changed
            && flag == Flag::Seen
            && let Some(f) = self.folder_mut(account, folder)
            && let Some(n) = &mut f.unseen
        {
            *n = if on {
                n.saturating_sub(1)
            } else {
                n.saturating_add(1)
            };
        }
    }

    /// The sidebar's rows again, the one picked kept.
    pub(super) fn rebuild(&mut self) {
        let picked = self.rows.get(self.row).copied();
        self.rows.clear();
        if self.live().count() > 1 {
            self.rows.push(Row::Everyone);
            self.rows.extend(TOGETHER.map(Row::All));
        }
        for a in self.live().collect::<Vec<_>>() {
            self.rows.push(Row::Account(a));
            self.rows
                .extend((0..self.accounts[a].folders.len()).map(|i| Row::Folder(a, i)));
        }
        if let Some(s) = &self.map.snapshot
            && !s.groups.is_empty()
        {
            self.rows.push(Row::MapHead);
            self.rows.push(Row::Map);
            self.rows
                .extend(s.groups.iter().map(|g| Row::Cluster(g.id)));
            if s.clustering.loner.iter().any(|&l| l) {
                self.rows.push(Row::Loners);
            }
        }
        if self.editable {
            self.rows.push(Row::Add);
        }
        self.row = picked
            .and_then(|p| self.rows.iter().position(|r| *r == p))
            .or_else(|| self.rows.iter().position(|r| matches!(r, Row::Folder(..))))
            .unwrap_or(0);
    }

    pub fn current(&self) -> Option<&Listed> {
        self.shown.get(self.list.selected()?)
    }

    /// The message picked: on the map, the map's, where it is looked at,
    /// read, or looked through in the Inbox; else in the list. Never the
    /// list's behind the map.
    pub(super) fn picked(&self) -> Option<Listed> {
        if self.map.mode == Mode::Map {
            return self.listed_dot(&self.dot()?);
        }
        self.current().cloned()
    }

    /// List what is picked in the sidebar, as it was if it was listed
    /// before, and ask for its newest messages.
    pub(super) fn open_folder(&mut self) {
        // What was marked in the list was of what it listed.
        if self.map.mode == Mode::List {
            self.map.region = None;
        }
        let view = match self.rows.get(self.row) {
            Some(&Row::Folder(a, i)) => {
                let Some(folder) = self.accounts[a].folders.get(i) else {
                    return;
                };
                if !folder.selectable {
                    return;
                }
                View::Folder(a, folder.path.clone())
            }
            Some(&Row::All(role)) => View::All(role),
            Some(&Row::Map) => View::Map,
            Some(&Row::Cluster(id)) => View::Cluster(id),
            Some(&Row::Loners) => View::Loners,
            _ => return,
        };
        self.open_view(view);
    }

    /// List `view`: a folder, a role's folders, the map's, or what was
    /// found.
    pub(super) fn open_view(&mut self, view: View) {
        if self.listing.as_ref().is_some_and(|l| l.view == view) {
            return;
        }
        if let Some(old) = self.listing.take() {
            for p in old.parts {
                self.listed
                    .insert((p.account, p.folder), (p.messages, p.total));
            }
        }
        self.listing = Some(Listing {
            view: view.clone(),
            parts: Vec::new(),
            fixed: None,
        });
        self.query.clear();
        self.found = None;
        self.searching = false;
        // What was picked before is not picked again: the list starts at
        // its top, and the message read is let go, not left beside a list
        // it is not in.
        self.shown.clear();
        self.list.select(Some(0));
        *self.list.offset_mut() = 0;
        self.reading = None;
        self.scroll = 0;
        self.due = None;
        let folders = match view {
            View::Folder(a, path) => vec![(a, path)],
            View::All(role) => self.folders_of(role),
            View::Map | View::Cluster(_) | View::Loners | View::Found(..) => {
                self.list_map();
                Vec::new()
            }
        };
        for (a, path) in folders {
            self.list_part(a, path);
        }
        self.narrow();
    }

    /// Open what is picked in the sidebar. In the map, a cluster narrows
    /// it, and a folder is listed in place of it, if `go` is; else as
    /// `open_folder` does.
    pub(super) fn open_row(&mut self, go: bool) {
        if self.map.mode == Mode::Map {
            match self.rows.get(self.row).copied() {
                Some(Row::Map) => return self.map.focused = Focused::All,
                Some(Row::Cluster(id)) => return self.map.focused = Focused::Cluster(id),
                Some(Row::Loners) => return self.map.focused = Focused::Loners,
                Some(Row::Folder(..) | Row::All(_)) if go => {
                    self.looked_now();
                    self.map.mode = Mode::List;
                }
                _ => return,
            }
        }
        self.open_folder();
    }

    /// What the map lists, if it is listed, as its snapshot now has it:
    /// every cluster, its unread first, each under its anchor; one; or what
    /// fits nowhere.
    pub(super) fn list_map(&mut self) {
        let Some(view) = self.listing.as_ref().map(|l| l.view.clone()) else {
            return;
        };
        let Some(s) = self.map.snapshot.clone() else {
            if let Some(l) = &mut self.listing {
                l.fixed = Some(Vec::new());
            }
            return;
        };
        let members: Vec<usize> = match view {
            View::Map => s
                .groups
                .iter()
                .flat_map(|g| {
                    let mut m = g.members.clone();
                    m.sort_by_key(|&i| (s.dots[i].flags.seen, std::cmp::Reverse(s.dots[i].date)));
                    m
                })
                .collect(),
            View::Cluster(id) => (0..s.dots.len()).filter(|&i| s.picked_by(id, i)).collect(),
            View::Loners => s.loners(),
            View::Found(ref words, ref within) => {
                let found = self.map.found_all(words);
                match within {
                    Within::Everywhere => found,
                    Within::Of { keys, .. } => found
                        .into_iter()
                        .filter(|&i| keys.contains(&s.dots[i].key))
                        .collect(),
                }
            }
            _ => return,
        };
        let mut members = members;
        if !matches!(view, View::Map) {
            members.sort_by_key(|&i| std::cmp::Reverse(s.dots[i].date));
        }
        let listed: Vec<Listed> = members
            .into_iter()
            .filter_map(|i| self.listed_dot(&s.dots[i]))
            .collect();
        if let Some(l) = &mut self.listing {
            l.fixed = Some(listed);
        }
    }

    /// The account of the map's message, by its place, and the folder it is
    /// in: the Inbox, if it is there.
    pub(super) fn place_of(&self, dot: &map::Dot) -> Option<(usize, String, u32)> {
        let a = self
            .live()
            .find(|&a| self.accounts[a].ident == dot.key.account)?;
        let place = dot
            .places
            .iter()
            .find(|p| self.inbox(a) == p.folder)
            .or_else(|| dot.places.first())?;
        Some((a, place.folder.clone(), place.uid))
    }

    /// The map's message as the list has it.
    pub(super) fn listed_dot(&self, dot: &map::Dot) -> Option<Listed> {
        let (a, folder, uid) = self.place_of(dot)?;
        let summary = Summary {
            uid,
            from: dot.from.clone(),
            address: dot.address.clone(),
            to: String::new(),
            subject: dot.subject.clone(),
            date: dot.date,
            flags: dot.flags,
            preview: dot.preview.clone(),
            size: dot.size,
            message_id: dot.message_id.clone(),
        };
        Some(Listed::new(a, &folder, summary))
    }

    /// List one account's folder in what is listed, as it was if it was
    /// listed before, and ask for its newest messages.
    pub(super) fn list_part(&mut self, a: usize, path: String) {
        let Some(listing) = &mut self.listing else {
            return;
        };
        let (messages, total) = self.listed.remove(&(a, path.clone())).unwrap_or_default();
        listing.parts.push(Part {
            account: a,
            folder: path.clone(),
            messages,
            total,
            loading: true,
            error: None,
        });
        let trash = self.is_trash(a, &path);
        self.ask_page(a, path, 0);
        if trash {
            self.sweep(a);
        }
    }

    /// Join the account's folder to the folders listed together, once its
    /// folders are known.
    pub(super) fn join_view(&mut self, a: usize) {
        let Some(View::All(role)) = self.listing.as_ref().map(|l| l.view.clone()) else {
            return;
        };
        let Some(path) = self.role_folder(a, role).map(|f| f.path.clone()) else {
            return;
        };
        if !self.listing.as_ref().is_some_and(|l| l.has(a, &path)) {
            self.list_part(a, path.clone());
            // What is searched for is searched in its folder too.
            if self.found.is_some() {
                self.ask_search(a, path);
            }
            self.narrow();
        }
    }

    /// Ask for a page of the folder's messages, `offset` from its newest.
    pub(super) fn ask_page(&mut self, account: usize, folder: String, offset: usize) {
        self.ask(
            account,
            Ask::Page {
                folder,
                offset,
                n: PAGE,
            },
        );
    }

    /// Ask the folder's server for what the query finds in it.
    pub(super) fn ask_search(&mut self, account: usize, folder: String) {
        let words = self.query.trim().to_string();
        if !words.is_empty() {
            self.ask(
                account,
                Ask::Search {
                    folder,
                    words,
                    n: FOUND,
                },
            );
        }
    }

    /// Look in the folder listed again, and count every folder's messages.
    pub fn refresh(&mut self) {
        self.refreshed = Instant::now();
        let parts: Vec<(usize, String)> = self
            .listing
            .iter_mut()
            .flat_map(|l| &mut l.parts)
            .map(|p| {
                p.loading = true;
                (p.account, p.folder.clone())
            })
            .collect();
        for (a, folder) in parts {
            self.ask_page(a, folder, 0);
        }
        for a in self.live().collect::<Vec<_>>() {
            self.map_pass(a);
            if matches!(self.accounts[a].state, State::Failed(_)) {
                self.accounts[a].state = State::Connecting;
                self.ask(a, Ask::Folders);
                continue;
            }
            let paths: Vec<String> = self.accounts[a]
                .folders
                .iter()
                .filter(|f| f.selectable)
                .map(|f| f.path.clone())
                .collect();
            for p in paths {
                self.ask(a, Ask::Count(p));
            }
        }
    }

    /// Show what the query finds: the server's answer, once asked, else the
    /// messages listed with every word of it in their sender, subject or
    /// preview.
    pub fn narrow(&mut self) {
        let picked = self.current().map(|m| (m.account, m.folder.clone(), m.uid));
        let words: Vec<String> = self
            .query
            .split_whitespace()
            .map(str::to_lowercase)
            .collect();
        self.shown = match (&self.found, &self.listing) {
            (Some(found), _) if !words.is_empty() => found.clone(),
            (
                _,
                Some(Listing {
                    fixed: Some(fixed), ..
                }),
            ) => fixed
                .iter()
                .filter(|m| {
                    words.is_empty() || {
                        let text = format!("{} {} {} {}", m.from, m.address, m.subject, m.preview)
                            .to_lowercase();
                        words.iter().all(|w| text.contains(w.as_str()))
                    }
                })
                .cloned()
                .collect(),
            (_, Some(l)) => merged(&l.parts)
                .into_iter()
                .filter(|(_, m)| {
                    words.is_empty() || {
                        let text = format!("{} {} {} {}", m.from, m.address, m.subject, m.preview)
                            .to_lowercase();
                        words.iter().all(|w| text.contains(w.as_str()))
                    }
                })
                .map(|(p, m)| Listed::new(p.account, &p.folder, m.clone()))
                .collect(),
            (_, None) => Vec::new(),
        };
        let at = picked
            .and_then(|(a, f, u)| self.shown.iter().position(|m| m.is(a, &f, u)))
            .or(self.list.selected())
            .unwrap_or(0);
        self.list
            .select((!self.shown.is_empty()).then(|| at.min(self.shown.len() - 1)));
    }

    /// Read the message picked in the list: at once, if it was read before.
    pub(super) fn read_picked(&mut self) {
        let Some(s) = self.current() else { return };
        let (a, folder, uid, seen) = (s.account, s.folder.clone(), s.uid, s.flags.seen);
        if self
            .reading
            .as_ref()
            .is_some_and(|r| r.account == a && r.folder == folder && r.uid == uid)
        {
            return;
        }
        self.scroll = 0;
        let mark = self.mark_read && !seen;
        let body = match self.read.get(&(a, folder.clone(), uid)) {
            Some(m) => {
                if mark {
                    self.set(Flag::Seen, true);
                }
                Body::Ready(m)
            }
            None => {
                self.ask(
                    a,
                    Ask::Open {
                        folder: folder.clone(),
                        uid,
                        mark,
                    },
                );
                Body::Loading
            }
        };
        self.reading = Some(Reading {
            account: a,
            folder,
            uid,
            body,
            on_map: false,
        });
    }

    /// Mark the message picked, or not, here and on its server.
    pub(super) fn set(&mut self, flag: Flag, on: bool) {
        let Some(s) = self.current() else { return };
        let (a, folder, uid) = (s.account, s.folder.clone(), s.uid);
        self.marked(a, &folder, uid, flag, on);
        self.ask(
            a,
            Ask::Set {
                folder,
                uid,
                flag,
                on,
            },
        );
    }

    pub(super) fn toggle(&mut self, flag: Flag) {
        let Some(s) = self.current() else { return };
        let on = match flag {
            Flag::Seen => !s.flags.seen,
            Flag::Flagged => !s.flags.flagged,
            Flag::Answered => !s.flags.answered,
        };
        self.set(flag, on);
    }

    /// The message read, if it has come.
    pub fn message(&self) -> Option<&Arc<Message>> {
        match &self.reading.as_ref()?.body {
            Body::Ready(m) => Some(m),
            _ => None,
        }
    }

    /// Ask for more of the folder's messages, if the one picked is near the
    /// end of those listed and there are more.
    pub(super) fn more(&mut self) {
        let near = self
            .list
            .selected()
            .is_some_and(|i| i + 10 >= self.shown.len());
        let Some(l) = &mut self.listing else { return };
        if !near || self.found.is_some() || !self.query.is_empty() {
            return;
        }
        // Those listed in part, but for those whose oldest listed is older
        // than the cut: they are not what holds the list back.
        let cut = cut(&l.parts);
        let asks: Vec<(usize, String, usize)> = l
            .parts
            .iter_mut()
            .filter(|p| !p.loading && p.partial())
            .filter(|p| cut.is_none_or(|c| p.oldest().is_none_or(|o| o >= c)))
            .map(|p| {
                p.loading = true;
                (p.account, p.folder.clone(), p.messages.len())
            })
            .collect();
        for (a, folder, offset) in asks {
            self.ask_page(a, folder, offset);
        }
    }

    pub(super) fn search(&mut self) {
        let Some(l) = &self.listing else { return };
        if self.query.trim().is_empty() {
            return;
        }
        // Of every folder of every account, from what letterbox knows: at
        // once, and never only the folder listed. The servers are asked
        // only where the map is not begun.
        if self.map.snapshot.is_some() {
            let words = self.query.trim().to_string();
            // A cluster listed, or what fits nowhere, is searched alone;
            // and a search again of what was found there, there again.
            let within = match &l.view {
                View::Cluster(_) | View::Loners => self.within(&l.view),
                View::Found(_, within) => within.clone(),
                _ => Within::Everywhere,
            };
            // What was listed before, for esc to go back to.
            if !matches!(l.view, View::Found(..)) {
                self.before_found = Some(l.view.clone());
            }
            let place = match &within {
                Within::Everywhere => None,
                Within::Of { name, .. } => Some(name.clone()),
            };
            self.open_view(View::Found(words.clone(), within));
            let n = self.shown.len();
            self.status = match (n, place) {
                (0, None) => format!("nothing found for {words}, in every folder letterbox knows"),
                (0, Some(place)) => format!("nothing found for {words} in {place}"),
                (n, None) => {
                    format!(
                        "{n} found in every folder, newest first · esc, or the sidebar, goes back"
                    )
                }
                (n, Some(place)) => {
                    format!("{n} found in {place}, newest first · esc, or the sidebar, goes back")
                }
            };
            return;
        }
        if self.map.mode == Mode::List {
            self.map.region = None;
        }
        let parts = l.keys();
        self.status = format!("searching {} on the server…", self.folder_name());
        for (a, folder) in parts {
            self.ask_search(a, folder);
        }
    }

    /// Move through the sidebar or the list, or scroll the message read.
    pub(super) fn step(&mut self, by: isize) {
        match self.focus {
            Focus::Side => {
                let mut at = self.row;
                loop {
                    let next = at.saturating_add_signed(by.signum());
                    if next == at || next >= self.rows.len() {
                        break;
                    }
                    at = next;
                    if matches!(
                        self.rows[at],
                        Row::All(_)
                            | Row::Folder(..)
                            | Row::Add
                            | Row::Map
                            | Row::Cluster(_)
                            | Row::Loners
                    ) {
                        self.row = at;
                        break;
                    }
                }
                self.due = Some((Instant::now(), Due::Folder));
            }
            Focus::List => self.pick(by),
            Focus::Reader => self.scroll_by(by),
        }
    }

    /// Pick another message in the list, `by` from the one picked, and read
    /// it once moving pauses, if it is shown beside the list.
    pub(super) fn pick(&mut self, by: isize) {
        if self.shown.is_empty() {
            return;
        }
        let at = self.list.selected().unwrap_or(0);
        self.list
            .select(Some(at.saturating_add_signed(by).min(self.shown.len() - 1)));
        if self.side_by_side {
            self.due = Some((Instant::now(), Due::Message));
        }
        self.more();
    }

    /// Read the next message in the list, or the one before.
    pub(super) fn next(&mut self, by: isize) {
        if self.shown.is_empty() {
            return;
        }
        let at = self.list.selected().unwrap_or(0);
        self.list
            .select(Some(at.saturating_add_signed(by).min(self.shown.len() - 1)));
        self.due = None;
        self.read_picked();
        self.more();
    }

    pub(super) fn scroll_by(&mut self, by: isize) {
        if self.reading.is_none() {
            return self.pick(by.signum());
        }
        let last = self.lines.saturating_sub(self.height);
        self.scroll = self.scroll.saturating_add_signed(by).min(last);
    }

    pub(super) fn jump(&mut self, end: bool) {
        match self.focus {
            Focus::Reader => {
                self.scroll = if end {
                    self.lines.saturating_sub(self.height)
                } else {
                    0
                }
            }
            Focus::List if !self.shown.is_empty() => {
                self.list
                    .select(Some(if end { self.shown.len() - 1 } else { 0 }));
                if self.side_by_side {
                    self.due = Some((Instant::now(), Due::Message));
                }
                self.more();
            }
            _ => {}
        }
    }
}
