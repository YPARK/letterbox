//! A label's folder split into subfolders by what its mail is about, or
//! removed, its mail filed where the map would file it: each shown first,
//! done only on y, and undone by C-x u.
use super::*;

/// A label split into subfolders, as shown before it is done: each part,
/// its name, to be typed over, its messages, and the words it was named
/// by; the part picked, and the name being typed, if one is.
#[derive(Clone, Debug)]
pub struct SplitPlan {
    pub label: String,
    pub parts: Vec<SplitPart>,
    pub at: usize,
    pub typing: Option<String>,
}

/// A part of a label split: what its subfolder is to be called, and what
/// goes there.
#[derive(Clone, Debug)]
pub struct SplitPart {
    pub name: String,
    pub words: Vec<String>,
    pub keys: Vec<crate::index::Key>,
}

/// A label removed, as shown before it is done: where its messages go,
/// each label they are voted to, or none, unfiled, to Archive.
#[derive(Clone, Debug)]
pub struct RemovePlan {
    pub label: String,
    pub to: Vec<(Option<String>, Vec<crate::index::Key>)>,
}

/// Where a message of a label goes: under a subfolder of it, by name; to
/// another label; or out of every label, unfiled.
#[derive(Clone, Debug, PartialEq, Eq)]
enum Dest {
    Sub(String),
    Label(String),
    Unfiled,
}

/// What a split or a removal asked of the servers, followed as each is
/// answered: for its progress to be said, and C-x u to put back each
/// where it was, by where it landed.
#[derive(Debug, Default)]
pub struct Refiling {
    /// Split or removed, and the label.
    verb: &'static str,
    label: String,
    total: usize,
    /// How many changes were asked, a message's move and its label each one.
    asked: usize,
    /// Moves asked, by account, folder and UID, and where each goes back.
    waiting: Vec<(usize, String, u32, String)>,
    /// Moved: by account, where it is now, by folder and UID, where it
    /// goes back, and its Message-ID.
    moved: Vec<(usize, String, u32, String, String)>,
    /// Labels changed where it is, as on Gmail: by account, folder, UID,
    /// what was taken off and what given.
    relabelled: Vec<Relabelled>,
    relabels_waiting: usize,
    /// Labels letterbox alone keeps, changed: the message, what was taken
    /// off and what given.
    kept: Vec<(crate::index::Key, Option<String>, Option<String>)>,
    /// Folders made for it, and deleted by it, by account and path; those
    /// still asked to be.
    made: Vec<(usize, String)>,
    deleted: Vec<(usize, String)>,
    to_make: Vec<(usize, String)>,
    to_delete: Vec<(usize, String)>,
    failed: Vec<String>,
}

impl Refiling {
    fn done(&self) -> usize {
        self.moved.len() + self.relabelled.len() + self.kept.len()
    }
}

/// A message's labels changed where it is: by account, folder and UID,
/// what was taken off and what given.
type Relabelled = (usize, String, u32, Option<String>, Option<String>);

/// The fewest a label holds to be split by what it is about.
const SPLIT_FROM: usize = 12;

impl App {
    /// The label of the cluster `id`, if it has one.
    fn label_of(&self, id: u32) -> Option<String> {
        self.map.snapshot.as_ref()?.group(id)?.label.clone()
    }

    /// The messages of the map in `label` itself, not in a folder under it,
    /// that can be moved out of it: in its folder on a server of folders,
    /// under it as a Gmail label or a keyword, or as letterbox alone keeps
    /// it.
    fn in_label(&self, label: &str) -> Vec<usize> {
        let Some(s) = &self.map.snapshot else {
            return Vec::new();
        };
        (0..s.dots.len())
            .filter(|&i| !self.routes(&s.dots[i], label).is_empty())
            .collect()
    }

    /// Each way the message `d` is under `label`, to come out of it by:
    /// moved from the folder it is in; its label, a Gmail label by its
    /// whole name or a keyword, taken off where it is; or the label
    /// letterbox keeps on it taken off.
    fn routes(&self, d: &map::Dot, label: &str) -> Vec<Route> {
        let Some(a) = self
            .live()
            .find(|&a| self.accounts[a].ident == d.key.account)
        else {
            return Vec::new();
        };
        let folded = crate::index::fold(label);
        let mut routes = Vec::new();
        let gmail = d.key.id.starts_with("gm:");
        if !gmail
            && let Some(f) = crate::index::labels::folder_for(&self.accounts[a].folders, label)
            && let Some(place) = d.places.iter().find(|p| p.folder == f.path)
        {
            routes.push(Route::Folder {
                a,
                folder: f.path.clone(),
                uid: place.uid,
                delimiter: f.delimiter.clone().unwrap_or_else(|| "/".to_string()),
            });
        }
        let index = self.index.lock().unwrap_or_else(|e| e.into_inner());
        let Some(record) = index.records.get(&d.key) else {
            return routes;
        };
        let leaf = |l: &str| {
            let name = crate::index::labels::unkeyword(l);
            let leaf = if gmail {
                name.rsplit('/').next().unwrap_or_default().to_string()
            } else {
                name.clone()
            };
            (crate::index::fold(&leaf) == folded).then_some(name)
        };
        if let Some(whole) = record.server.iter().find_map(|l| leaf(l))
            && let Some(place) = d.places.first()
        {
            routes.push(Route::Labels {
                a,
                folder: place.folder.clone(),
                uid: place.uid,
                whole,
                gmail,
            });
        }
        if index
            .keep
            .labels
            .get(&d.key)
            .is_some_and(|ls| ls.iter().any(|l| crate::index::fold(l) == folded))
        {
            routes.push(Route::Kept);
        }
        routes
    }

    /// Show how the label of the cluster `id` would be split: its mail in
    /// parts by what it is about, each a subfolder named by the words most
    /// its own.
    pub(super) fn ask_split(&mut self, id: u32) {
        match self.label_of(id) {
            Some(label) => self.ask_split_label(&label),
            None => self.status = "no label to split".to_string(),
        }
    }

    /// Show how `label` would be split.
    pub fn ask_split_label(&mut self, label: &str) {
        let label = label.to_string();
        let Some(s) = self.map.snapshot.clone() else {
            return;
        };
        let members = self.in_label(&label);
        if members.len() < SPLIT_FROM {
            self.status = format!(
                "{label} holds {} of its own: too few to split",
                members.len()
            );
            return;
        }
        let (parts, _) = crate::model::parts(&s.model, &members, None, map::SEED);
        if parts.len() < 2 {
            self.status = format!("{label} is all about one thing: nothing to split it by");
            return;
        }
        let without = crate::model::words(&label);
        let mut names: Vec<String> = Vec::new();
        let parts = parts
            .into_iter()
            .enumerate()
            .map(|(k, part)| {
                let words = crate::model::part_words(&s.model, &part, &members, &without);
                let mut name = match words.as_slice() {
                    [] => format!("Part {}", k + 1),
                    w => capitalised(&w.join(" ")),
                };
                while names.iter().any(|n| n.eq_ignore_ascii_case(&name)) {
                    name.push_str(&format!(" {}", k + 1));
                }
                names.push(name.clone());
                SplitPart {
                    name,
                    words,
                    keys: part.iter().map(|&i| s.dots[i].key.clone()).collect(),
                }
            })
            .collect();
        self.dialog = Dialog::Split(Box::new(SplitPlan {
            label,
            parts,
            at: 0,
            typing: None,
        }));
    }

    /// Show where the mail of the label of the cluster `id` would go, were
    /// it removed: each where the map would file it, were there no such
    /// label, else unfiled, to Archive.
    pub(super) fn ask_remove(&mut self, id: u32) {
        match self.label_of(id) {
            Some(label) => self.ask_remove_label(&label),
            None => self.status = "no label to remove".to_string(),
        }
    }

    /// Show where the mail of `label` would go, were it removed.
    pub fn ask_remove_label(&mut self, label: &str) {
        let label = label.to_string();
        let Some(s) = self.map.snapshot.clone() else {
            return;
        };
        // A folder under it would be removed with it: never.
        for a in self.live().collect::<Vec<_>>() {
            let folders = &self.accounts[a].folders;
            if let Some(f) = crate::index::labels::folder_for(folders, &label) {
                let under = format!("{}{}", f.path, f.delimiter.as_deref().unwrap_or("/"));
                if folders.iter().any(|g| g.path.starts_with(&under)) {
                    self.status =
                        format!("{label} has folders under it: remove those first, or split it");
                    return;
                }
            }
        }
        let members = self.in_label(&label);
        let not_it = self
            .index
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .keep
            .not_it
            .clone();
        let voted = s.model.voted_without(&label, &members, |i, l| {
            not_it.contains(&(s.model.keys[i].clone(), l.to_string()))
        });
        let mut to: std::collections::BTreeMap<Option<String>, Vec<crate::index::Key>> =
            Default::default();
        for (&i, v) in members.iter().zip(voted) {
            to.entry(v).or_default().push(s.dots[i].key.clone());
        }
        let mut to: Vec<(Option<String>, Vec<crate::index::Key>)> = to.into_iter().collect();
        // Most first, unfiled last.
        to.sort_by(|a, b| {
            a.0.is_none()
                .cmp(&b.0.is_none())
                .then(b.1.len().cmp(&a.1.len()))
                .then(a.0.cmp(&b.0))
        });
        self.dialog = Dialog::RemoveLabel(Box::new(RemovePlan { label, to }));
    }

    /// A key in the split shown: ↑ ↓ pick a part, ⏎ types its name over,
    /// ⏎ again keeps it; y splits it so; any other key leaves it as it is.
    pub(super) fn split_key(&mut self, key: KeyEvent) {
        let Dialog::Split(plan) = &mut self.dialog else {
            return;
        };
        if let Some(typed) = &mut plan.typing {
            match key.code {
                KeyCode::Enter => {
                    let name = typed.trim().to_string();
                    if !name.is_empty() && !name.contains('/') {
                        plan.parts[plan.at].name = name;
                    }
                    plan.typing = None;
                }
                KeyCode::Esc => plan.typing = None,
                _ => {
                    edit_line(typed, key);
                }
            }
            return;
        }
        let last = plan.parts.len().saturating_sub(1);
        match (keymap::act(key), key.code) {
            (Some(Act::Down), _) => plan.at = (plan.at + 1).min(last),
            (Some(Act::Up), _) => plan.at = plan.at.saturating_sub(1),
            (_, KeyCode::Enter) => plan.typing = Some(plan.parts[plan.at].name.clone()),
            (_, KeyCode::Char('y' | 'Y')) if key.modifiers.is_empty() => {
                let Dialog::Split(plan) = std::mem::take(&mut self.dialog) else {
                    unreachable!()
                };
                self.split_now(*plan);
            }
            _ => {
                let label = plan.label.clone();
                self.dialog = Dialog::Closed;
                self.status = format!("{label} left as it is");
            }
        }
    }

    /// A key in the removal shown: y removes it so; any other key leaves
    /// it as it is.
    pub(super) fn remove_label_key(&mut self, key: KeyEvent) {
        let Dialog::RemoveLabel(plan) = std::mem::take(&mut self.dialog) else {
            return;
        };
        if matches!(key.code, KeyCode::Char('y' | 'Y')) && key.modifiers.is_empty() {
            self.remove_now(*plan);
        } else {
            self.status = format!("{} left as it is", plan.label);
        }
    }

    /// Split `plan.label` as shown: each part's subfolder made, where it
    /// is a folder, and its mail moved there.
    fn split_now(&mut self, plan: SplitPlan) {
        let total = plan.parts.iter().map(|p| p.keys.len()).sum();
        self.begin_refiling("split", &plan.label, total);
        for part in &plan.parts {
            for key in &part.keys {
                self.refile(key, &plan.label, &Dest::Sub(part.name.clone()));
            }
        }
        self.say_refiling();
    }

    /// Remove `plan.label` as shown: each of its messages filed where it
    /// was shown to go, and then its folder deleted, once its server finds
    /// it empty.
    fn remove_now(&mut self, plan: RemovePlan) {
        let total = plan.to.iter().map(|(_, k)| k.len()).sum();
        self.begin_refiling("removed", &plan.label, total);
        for (to, keys) in &plan.to {
            let dest = to.clone().map_or(Dest::Unfiled, Dest::Label);
            for key in keys {
                self.refile(key, &plan.label, &dest);
            }
        }
        // Asked after the moves, each account's asked in turn: deleted
        // only once its folder is empty, never with mail in it.
        for a in self.live().collect::<Vec<_>>() {
            if let Some(f) =
                crate::index::labels::folder_for(&self.accounts[a].folders, &plan.label)
            {
                let path = f.path.clone();
                self.refiling.to_delete.push((a, path.clone()));
                self.ask(a, Ask::DeleteFolder { path });
            }
        }
        self.say_refiling();
    }

    fn begin_refiling(&mut self, verb: &'static str, label: &str, total: usize) {
        self.refiling = Refiling {
            verb,
            label: label.to_string(),
            total,
            ..Refiling::default()
        };
        self.last = Some(Last::Refile);
    }

    /// The message `key` out of `label`, to `dest`, the way its account
    /// keeps labels.
    fn refile(&mut self, key: &crate::index::Key, label: &str, dest: &Dest) {
        let Some(d) = self
            .map
            .snapshot
            .as_ref()
            .and_then(|s| s.find(key).map(|i| s.dots[i].clone()))
        else {
            self.refiling
                .failed
                .push(format!("{}: not on the map", key.id));
            return;
        };
        let routes = self.routes(&d, label);
        if routes.is_empty() {
            self.refiling
                .failed
                .push(format!("{}: not in {label} itself", d.subject));
            return;
        }
        for route in routes {
            self.refiling.asked += 1;
            self.refile_by(key, &d, label, dest, route);
        }
    }

    /// The message `d` out of `label` to `dest`, by one way it is under it.
    fn refile_by(
        &mut self,
        key: &crate::index::Key,
        d: &map::Dot,
        label: &str,
        dest: &Dest,
        route: Route,
    ) {
        match route {
            Route::Folder {
                a,
                folder,
                uid,
                delimiter,
            } => {
                let to = match dest {
                    Dest::Sub(name) => format!(
                        "{folder}{delimiter}{}",
                        crate::index::labels::folder_name(name)
                    ),
                    Dest::Label(l) => {
                        match crate::index::labels::folder_for(&self.accounts[a].folders, l) {
                            Some(f) => f.path.clone(),
                            None => crate::index::labels::folder_name(l),
                        }
                    }
                    Dest::Unfiled => self.accounts[a]
                        .folders
                        .iter()
                        .find(|f| f.role == Role::Archive && f.selectable)
                        .map_or_else(|| "Archive".to_string(), |f| f.path.clone()),
                };
                self.make_folder(a, &to);
                self.refiling
                    .waiting
                    .push((a, folder.clone(), uid, folder.clone()));
                self.ask(
                    a,
                    Ask::Move {
                        from: folder,
                        uid,
                        to,
                        message_id: d.message_id.clone(),
                        trashed: false,
                    },
                );
            }
            Route::Labels {
                a,
                folder,
                uid,
                whole,
                gmail,
            } => {
                // Under a sub-label: on Gmail its whole name nested under
                // this one's; a keyword beside its parent, as a folder's
                // mail carries both.
                let (off, on) = match dest {
                    Dest::Sub(name) if gmail => {
                        let sub = format!("{whole}/{name}");
                        self.make_folder(a, &sub);
                        (Some(whole), Some(sub))
                    }
                    Dest::Sub(name) => (None, Some(name.clone())),
                    Dest::Label(l) => (Some(whole), Some(l.clone())),
                    Dest::Unfiled => (Some(whole), None),
                };
                self.refiling.relabels_waiting += 1;
                self.ask(
                    a,
                    Ask::Relabel {
                        folder,
                        uid,
                        off,
                        on,
                    },
                );
            }
            Route::Kept => {
                let on = match dest {
                    Dest::Sub(name) => Some(name.clone()),
                    Dest::Label(l) => Some(l.clone()),
                    Dest::Unfiled => None,
                };
                // A sub-label kept beside its parent, as a folder's mail
                // carries both; else the label taken off.
                let off = (!matches!(dest, Dest::Sub(_))).then(|| label.to_string());
                if self.keep_labels(key, off.as_deref(), on.as_deref()) {
                    self.refiling.kept.push((key.clone(), off, on));
                }
            }
        }
    }

    /// Make the folder `path` on account `a`, once, before what moves
    /// there.
    fn make_folder(&mut self, a: usize, path: &str) {
        let there = self.accounts[a].folders.iter().any(|f| f.path == path);
        let asked = self
            .refiling
            .to_make
            .iter()
            .any(|(x, p)| *x == a && p == path);
        if there || asked {
            return;
        }
        self.refiling.to_make.push((a, path.to_string()));
        self.ask(
            a,
            Ask::CreateFolder {
                path: path.to_string(),
            },
        );
    }

    /// Labels letterbox alone keeps on `key`: `off` taken off, `on` given;
    /// whether it was written.
    fn keep_labels(
        &mut self,
        key: &crate::index::Key,
        off: Option<&str>,
        on: Option<&str>,
    ) -> bool {
        let mut events = Vec::new();
        if let Some(l) = on {
            events.push(crate::index::Kept::Label {
                key: key.clone(),
                label: l.to_string(),
                on: true,
            });
        }
        if let Some(l) = off {
            events.push(crate::index::Kept::Label {
                key: key.clone(),
                label: l.to_string(),
                on: false,
            });
        }
        let kept = self
            .index
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .keep(events);
        match kept {
            Ok(()) => true,
            Err(e) => {
                self.refiling
                    .failed
                    .push(format!("cannot keep the label: {e:#}"));
                false
            }
        }
    }

    /// Say how far what was split or removed has come.
    fn say_refiling(&mut self) {
        let r = &self.refiling;
        let mut said = format!(
            "{} {}: {} of {} done, of {} messages",
            r.label,
            r.verb,
            r.done(),
            r.asked,
            r.total
        );
        if !r.failed.is_empty() {
            said.push_str(&format!(" · {} not: {}", r.failed.len(), r.failed[0]));
        }
        said.push_str(" · C-x u puts it back");
        self.status = said;
    }

    /// A move answered: where it landed, if it was of what is split or
    /// removed.
    pub(super) fn refile_moved(
        &mut self,
        a: usize,
        from: &str,
        uid: u32,
        landed: Result<(String, u32, String), String>,
    ) {
        let Some(at) = self
            .refiling
            .waiting
            .iter()
            .position(|(x, f, u, _)| *x == a && f == from && *u == uid)
        else {
            return;
        };
        let (_, _, _, back) = self.refiling.waiting.remove(at);
        match landed {
            Ok((to, new_uid, message_id)) => {
                self.refiling.moved.push((a, to, new_uid, back, message_id))
            }
            Err(e) => self.refiling.failed.push(e),
        }
        self.say_refiling();
    }

    /// A Gmail message's labels changed, as answered.
    pub(super) fn refile_relabelled(
        &mut self,
        a: usize,
        folder: String,
        uid: u32,
        off: Option<String>,
        on: Option<String>,
        result: Result<(), String>,
    ) {
        if self.refiling.relabels_waiting == 0 {
            return;
        }
        self.refiling.relabels_waiting -= 1;
        match result {
            Ok(()) => self.refiling.relabelled.push((a, folder, uid, off, on)),
            Err(e) => self.refiling.failed.push(e),
        }
        self.say_refiling();
    }

    /// A folder made or deleted, as answered, if it was asked for what is
    /// split or removed.
    pub(super) fn refile_folder(
        &mut self,
        a: usize,
        path: &str,
        made: bool,
        result: Result<(), String>,
    ) {
        // The folders as they are now, whoever asked.
        self.ask(a, Ask::Folders);
        let (asked, done) = if made {
            (&mut self.refiling.to_make, &mut self.refiling.made)
        } else {
            (&mut self.refiling.to_delete, &mut self.refiling.deleted)
        };
        let Some(at) = asked.iter().position(|(x, p)| *x == a && p == path) else {
            return;
        };
        let (a, path) = asked.remove(at);
        match result {
            Ok(()) => done.push((a, path)),
            Err(e) => self.refiling.failed.push(format!("{path}: {e}")),
        }
        self.say_refiling();
    }

    /// C-x u: what was split or removed put back, each where it was: the
    /// folders deleted made again first, each message moved or relabelled
    /// back, and the folders made for it deleted, once empty.
    pub(super) fn undo_refiling(&mut self) {
        let r = std::mem::take(&mut self.refiling);
        for (a, path) in &r.deleted {
            self.ask(*a, Ask::CreateFolder { path: path.clone() });
        }
        let back = r.moved.len() + r.relabelled.len() + r.kept.len();
        for (a, at, uid, to, message_id) in r.moved {
            self.ask(
                a,
                Ask::Move {
                    from: at,
                    uid,
                    to,
                    message_id,
                    trashed: false,
                },
            );
        }
        for (a, folder, uid, off, on) in r.relabelled {
            self.ask(
                a,
                Ask::Relabel {
                    folder,
                    uid,
                    off: on,
                    on: off,
                },
            );
        }
        for (key, off, on) in r.kept {
            let _ = self.keep_labels(&key, on.as_deref(), off.as_deref());
        }
        for (a, path) in r.made {
            self.ask(a, Ask::DeleteFolder { path });
        }
        self.status = format!("{} put back as it was: {back} moved back", r.label);
        if !r.waiting.is_empty() || r.relabels_waiting > 0 {
            self.status
                .push_str(" · some were still being moved: look again, and C-x u");
        }
    }
}

/// How a message comes out of a label, by how its account keeps labels.
#[derive(Clone, Debug)]
enum Route {
    /// Moved from the folder that is the label, on account `a`.
    Folder {
        a: usize,
        folder: String,
        uid: u32,
        delimiter: String,
    },
    /// Its label changed where it is: a Gmail label, by its whole name,
    /// or a keyword.
    Labels {
        a: usize,
        folder: String,
        uid: u32,
        whole: String,
        gmail: bool,
    },
    /// The label letterbox alone keeps on it, changed.
    Kept,
}
