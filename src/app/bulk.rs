//! What is selected, with the mouse, and acted on together only once
//! asked: more than one message trashed or archived is said first, by
//! account and folder, mail in an Inbox and mail you sent left out of a
//! trashing unless taken; and what was trashed together put back by C-x u.
use super::*;

/// Over this many marked, the foot says so in the flag's colour.
pub const MARK_BIG: usize = 20;

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum BulkAct {
    Trash,
    Archive,
    /// Filed under the label named.
    File(String),
    /// Moved to another account's folder: copied there, then the original
    /// moved to Trash.
    Elsewhere(usize, String),
}

/// What is asked of several messages, held as it was when asked: what is
/// done, to which, whether the Inbox's mail and the mail you sent are
/// taken too, trashed, and how it is said.
#[derive(Clone, Debug)]
pub struct Bulk {
    pub act: BulkAct,
    pub keys: Vec<crate::index::Key>,
    pub inbox: bool,
    pub sent: bool,
    pub said: BulkSaid,
}

/// What the pop-up says of a bulk act: its title, what it takes, by
/// account and folder, and what is left out, with the key that takes it.
#[derive(Clone, Debug, Default)]
pub struct BulkSaid {
    pub title: String,
    pub lines: Vec<String>,
    pub warnings: Vec<String>,
}

impl App {
    /// Ctrl and a click on a row: it added to what is selected, or taken
    /// away.
    pub(super) fn select_one(&mut self, row: Target) {
        match row {
            Target::Message(i) => {
                self.focus = Focus::List;
                self.list.select(Some(i));
                self.list_mark();
            }
            Target::ReviewRow(r) => self.review_toggle(r),
            _ => {}
        }
    }

    /// Shift and a click on a row: selected from the row picked as far as
    /// it.
    pub(super) fn select_as_far_as(&mut self, row: Target) {
        match row {
            Target::Message(i) => {
                let from = self.list.selected().unwrap_or(i);
                self.list_select(from, i);
            }
            Target::ReviewRow(r) => {
                let from = self.review_at().map_or(r, |(_, at)| at);
                self.review_select(from, r);
            }
            _ => {}
        }
    }

    /// Dragged from one row to another, of the same pane: both selected,
    /// and those between; on the row pressed on, nothing.
    pub(super) fn select_between(&mut self, from: Target, to: Target) {
        match (from, to) {
            (Target::Message(a), Target::Message(b)) if a != b => self.list_select(a, b),
            (Target::ReviewRow(a), Target::ReviewRow(b)) if a != b => self.review_select(a, b),
            _ => {}
        }
    }

    /// Trash or archive what is marked: at once if it is one message, else
    /// asked first.
    pub(super) fn ask_bulk(&mut self, act: BulkAct) -> bool {
        let keys: Vec<crate::index::Key> = self.region_dots().into_iter().map(|d| d.key).collect();
        if keys.len() <= 1 {
            return match act {
                BulkAct::Archive => self.region_archive(true),
                _ => self.region_trash(&|_| true),
            };
        }
        self.open_bulk(act, keys);
        true
    }

    /// Ask whether to do `act` to the messages `keys`, all of them.
    pub(super) fn open_bulk(&mut self, act: BulkAct, keys: Vec<crate::index::Key>) {
        let mut bulk = Bulk {
            act,
            keys,
            inbox: false,
            sent: false,
            said: BulkSaid::default(),
        };
        bulk.said = self.bulk_said(&bulk);
        self.dialog = Dialog::Bulk(Box::new(bulk));
    }

    /// The messages `keys`, as the map has them.
    fn dots_of(&self, keys: &[crate::index::Key]) -> Vec<map::Dot> {
        let Some(s) = &self.map.snapshot else {
            return Vec::new();
        };
        keys.iter()
            .filter_map(|k| s.find(k).map(|i| s.dots[i].clone()))
            .collect()
    }

    /// Whether `bulk` takes the message `d`.
    fn takes(bulk: &Bulk, d: &map::Dot) -> bool {
        match bulk.act {
            BulkAct::Archive => d.system.inbox,
            BulkAct::Trash => (bulk.inbox || !d.system.inbox) && (bulk.sent || !d.system.sent),
            BulkAct::File(_) | BulkAct::Elsewhere(..) => true,
        }
    }

    /// What the pop-up for `bulk` says.
    fn bulk_said(&self, bulk: &Bulk) -> BulkSaid {
        let dots = self.dots_of(&bulk.keys);
        let taken: Vec<&map::Dot> = dots.iter().filter(|d| Self::takes(bulk, d)).collect();
        let mut by: std::collections::BTreeMap<String, std::collections::BTreeMap<String, usize>> =
            Default::default();
        let mut no_trash = 0;
        for d in &taken {
            let Some((a, folder, _)) = self.place_of(d) else {
                continue;
            };
            if bulk.act == BulkAct::Trash && self.trash_of(a).is_none() {
                no_trash += 1;
            }
            *by.entry(self.accounts[a].name.clone())
                .or_default()
                .entry(self.folder_label(a, &folder))
                .or_default() += 1;
        }
        let lines = by
            .into_iter()
            .map(|(account, folders)| {
                let folders: Vec<String> = folders
                    .into_iter()
                    .map(|(f, n)| format!("{f} {n}"))
                    .collect();
                format!("{account}  {}", folders.join(" · "))
            })
            .collect();
        let mut warnings = Vec::new();
        if bulk.act == BulkAct::Trash {
            let inbox = dots.iter().filter(|d| d.system.inbox).count();
            let sent = dots.iter().filter(|d| d.system.sent).count();
            if inbox > 0 {
                warnings.push(if bulk.inbox {
                    format!("{inbox} in an Inbox: taken too · i leaves them out")
                } else {
                    format!("{inbox} in an Inbox: left out · i takes them too")
                });
            }
            if sent > 0 {
                warnings.push(if bulk.sent {
                    format!("{sent} you sent: taken too · s leaves them out")
                } else {
                    format!("{sent} you sent: left out · s takes them too")
                });
            }
            if no_trash > 0 {
                warnings.push(format!(
                    "{no_trash} of an account with no Trash known: not moved · d on one alone asks which"
                ));
            }
        }
        if let BulkAct::Elsewhere(..) = bulk.act {
            warnings.push(
                "each is copied there, then moved to Trash here, once the copy is found"
                    .to_string(),
            );
        }
        let n = taken.len();
        let title = match &bulk.act {
            BulkAct::Trash => format!("Move {n} to Trash?"),
            BulkAct::Archive => format!("Archive {n}?"),
            BulkAct::File(label) => format!("File {n} under {label}?"),
            BulkAct::Elsewhere(to, path) => {
                let account = self.accounts.get(*to).map_or("", |a| a.name.as_str());
                format!("Move {n} to {path} · {account}?")
            }
        };
        BulkSaid {
            title,
            lines,
            warnings,
        }
    }

    /// A key in the pop-up of a bulk act: `i` and `s` take the Inbox's
    /// mail or yours sent, or leave it out; `y` does it; any other key
    /// does nothing, what is marked kept.
    pub(super) fn bulk_key(&mut self, key: KeyEvent) {
        let Dialog::Bulk(bulk) = &mut self.dialog else {
            return;
        };
        let plain = key.modifiers.is_empty() || key.modifiers == KeyModifiers::SHIFT;
        let trash = bulk.act == BulkAct::Trash;
        match key.code {
            KeyCode::Char('i') if plain && trash => {
                bulk.inbox = !bulk.inbox;
                let said = self.bulk_said_of_dialog();
                if let Dialog::Bulk(b) = &mut self.dialog {
                    b.said = said;
                }
            }
            KeyCode::Char('s') if plain && trash => {
                bulk.sent = !bulk.sent;
                let said = self.bulk_said_of_dialog();
                if let Dialog::Bulk(b) = &mut self.dialog {
                    b.said = said;
                }
            }
            KeyCode::Char('y' | 'Y') if plain => {
                let Dialog::Bulk(bulk) = std::mem::take(&mut self.dialog) else {
                    unreachable!()
                };
                let dots = self.dots_of(&bulk.keys);
                let keys: Vec<crate::index::Key> = dots
                    .iter()
                    .filter(|d| Self::takes(&bulk, d))
                    .map(|d| d.key.clone())
                    .collect();
                if keys.is_empty() {
                    self.status = match bulk.act {
                        BulkAct::Trash => "nothing done: all of it is in an Inbox or yours sent · i or s takes them",
                        BulkAct::Archive => "nothing done: none of it is in an Inbox",
                        _ => "nothing done: none of it is on the map",
                    }
                    .to_string();
                    return;
                }
                // Done together, as a cluster is: nothing is said of each,
                // so nothing is learnt.
                match bulk.act {
                    BulkAct::Trash => {
                        self.map.region = Some(map::Region { keys });
                        self.region_trash(&|_| false);
                    }
                    BulkAct::Archive => {
                        self.map.region = Some(map::Region { keys });
                        self.region_archive(false);
                    }
                    BulkAct::File(label) => {
                        self.map.region = None;
                        self.file_now(keys, &label, false);
                    }
                    BulkAct::Elsewhere(to, path) => {
                        self.map.region = None;
                        for d in dots.iter().filter(|d| keys.contains(&d.key)) {
                            self.bring(d, to, &path, None);
                        }
                    }
                }
            }
            _ => {
                self.dialog = Dialog::Closed;
                self.status = "nothing done · what is marked is kept, esc lets it go".to_string();
            }
        }
    }

    /// What the bulk pop-up open says, said again.
    fn bulk_said_of_dialog(&self) -> BulkSaid {
        match &self.dialog {
            Dialog::Bulk(b) => self.bulk_said(b),
            _ => BulkSaid::default(),
        }
    }
}
