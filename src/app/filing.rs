//! Filing under labels: from the list, marked or one, where the map
//! suggests, and labels renamed.
use super::*;

impl App {
    /// File the messages `keys` under `label`: shown at once as given, and
    /// each asked of its server; the label's suggestions shown after.
    pub(super) fn file_keys(&mut self, keys: Vec<crate::index::Key>, label: &str) {
        let label = label.trim().to_string();
        self.map.moving = None;
        // Picked for a message looked through: chosen, to be done with the
        // rest.
        if let Some(r) = &mut self.review
            && let Some(k) = r.relabel.take()
            && keys == [k.clone()]
            && !label.is_empty()
        {
            r.chosen.insert(k, Choice::Move(label.clone()));
            self.status = format!("to go to {label} · $ does it, with the rest");
            self.focus = Focus::Reader;
            return;
        }
        if label.is_empty() || keys.is_empty() {
            self.status = "not filed".to_string();
            return;
        }
        let Some(s) = self.map.snapshot.clone() else {
            return;
        };
        // Filed, it is kept: learnt as not thrown away.
        self.teach(&keys, false);
        let at = jiff::Timestamp::now().as_second();
        let mut filed = 0;
        let mut pending = Vec::new();
        let mut asks = Vec::new();
        // Filed from a folder's list, it is moved from that folder.
        let listed = match self.listing.as_ref().map(|l| &l.view) {
            Some(View::Folder(a, path)) => Some((*a, path.clone())),
            _ => None,
        };
        for key in &keys {
            let Some(i) = s.find(key) else { continue };
            let dot = &s.dots[i];
            let Some((a, folder, uid)) = self.place_of(dot) else {
                continue;
            };
            pending.push(crate::index::Kept::Pending {
                key: key.clone(),
                label: label.clone(),
                on: true,
                at,
            });
            asks.push((
                a,
                Ask::File {
                    folder,
                    uid,
                    label: label.clone(),
                    message_id: dot.message_id.clone(),
                    key: key.clone(),
                    from: listed
                        .as_ref()
                        .filter(|(at, _)| *at == a)
                        .map(|(_, path)| path.clone()),
                },
            ));
            filed += 1;
        }
        let kept = self
            .index
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .keep(pending);
        if let Err(e) = kept {
            self.status = format!("cannot file it: {e:#}");
            return;
        }
        // Out of the Inbox, out of its list, and no longer read, before
        // the server has said so, as archived mail is.
        let inbox: Vec<(usize, String, u32)> = asks
            .iter()
            .filter_map(|(a, ask)| match ask {
                Ask::File { folder, uid, .. } if *folder == self.inbox(*a) => {
                    Some((*a, folder.clone(), *uid))
                }
                _ => None,
            })
            .collect();
        for (a, ask) in asks {
            self.ask(a, ask);
        }
        self.drop_all_listed(&inbox);
        self.map.region = None;
        self.map.suggesting = Some(label.clone());
        self.status = format!(
            "{} under {label}, out of the Inbox{}",
            if filed == 1 {
                "filed".to_string()
            } else {
                format!("{filed} filed")
            },
            if self.map.mode == Mode::Map {
                " · y files the next it would go with too"
            } else {
                ""
            }
        );
    }

    /// Mark the message picked in the list, or let it go, and go on to the
    /// next: what is marked is acted on together.
    pub(super) fn list_mark(&mut self) {
        let Some(key) = self
            .listed_on_map()
            .and_then(|i| self.map.snapshot.as_ref().map(|s| s.dots[i].key.clone()))
        else {
            self.status = "not on the map yet: it is marked once the map has read it".to_string();
            return;
        };
        let region = self
            .map
            .region
            .get_or_insert_with(|| map::Region { keys: Vec::new() });
        match region.keys.iter().position(|k| *k == key) {
            Some(at) => {
                region.keys.remove(at);
            }
            None => region.keys.push(key),
        }
        let n = region.keys.len();
        if n == 0 {
            self.map.region = None;
        }
        self.status = format!(
            "{n} marked: d trashes them, e archives them, m moves them, u and s mark them, esc lets them go"
        );
        self.step(1);
    }

    /// Mark from the row Shift and the arrows began at to the one picked,
    /// `by` past it, with what was marked before: back, fewer.
    pub(super) fn list_extend(&mut self, by: isize) {
        let Some(at) = self.list.selected() else {
            return;
        };
        let before = self.map.region.as_ref().map(|r| r.keys.clone());
        let (from, mut keys) = self
            .marking
            .get_or_insert_with(|| (at, before.unwrap_or_default()))
            .clone();
        self.step(by);
        let to = self.list.selected().unwrap_or(at);
        for l in &self.shown[from.min(to)..=from.max(to).min(self.shown.len().saturating_sub(1))] {
            if let Some(key) = self.listed_key(l)
                && !keys.contains(&key)
            {
                keys.push(key);
            }
        }
        let n = keys.len();
        self.map.region = (n > 0).then_some(map::Region { keys });
        self.status = format!(
            "{n} marked: d trashes them, e archives them, m moves them, u and s mark them, esc lets them go"
        );
    }

    /// What the index knows the message listed `l` by, once the map has
    /// read it.
    pub(super) fn listed_key(&self, l: &Listed) -> Option<crate::index::Key> {
        let ident = &self.accounts.get(l.account)?.ident;
        self.index
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .key_at(ident, &l.folder, l.summary.uid)
    }

    /// What a key does to what is marked in the list; whether it was its
    /// to do.
    pub(super) fn list_marked_act(&mut self, act: Act) -> bool {
        let Some(keys) = self.map.region.as_ref().map(|r| r.keys.clone()) else {
            return false;
        };
        match act {
            Act::Seen => self.region_flag(Flag::Seen),
            Act::Flag => self.region_flag(Flag::Flagged),
            Act::Trash => self.region_trash(),
            Act::Archive => self.region_archive(),
            Act::Back => {
                self.map.region = None;
                self.status = "let go".to_string();
                true
            }
            Act::Move => {
                let Some(i) = keys
                    .first()
                    .and_then(|k| self.map.snapshot.as_ref()?.find(k))
                else {
                    return false;
                };
                self.open_menu(i);
                if let Some(menu) = &mut self.menu {
                    menu.with = keys[1..].to_vec();
                }
                true
            }
            Act::NewLabel => {
                self.map.moving = Some(keys);
                self.ask_new_label();
                true
            }
            _ => false,
        }
    }

    /// Mark every message listed that would go where the one picked would;
    /// marked so already, file them all there.
    pub(super) fn list_file_all(&mut self) {
        let Some(i) = self.listed_on_map() else {
            self.status = "not on the map yet: it is filed once the map has read it".to_string();
            return;
        };
        let Some(s) = self.map.snapshot.clone() else {
            return;
        };
        let Some(label) = s.suggestion(i).map(str::to_string) else {
            self.status = "nowhere it would go: m picks where".to_string();
            return;
        };
        let keys: Vec<crate::index::Key> = self
            .shown
            .iter()
            .filter_map(|l| {
                let ident = &self.accounts.get(l.account)?.ident;
                let key = self
                    .index
                    .lock()
                    .unwrap_or_else(|e| e.into_inner())
                    .key_at(ident, &l.folder, l.summary.uid)?;
                let j = s.find(&key)?;
                (s.suggestion(j) == Some(label.as_str())).then_some(key)
            })
            .collect();
        if self.map.region.as_ref().is_some_and(|r| r.keys == keys) {
            self.file_keys(keys, &label);
            self.map.region = None;
        } else {
            self.status = format!(
                "{} would go to {label}, marked: Y files them, esc lets go",
                keys.len()
            );
            self.map.region = Some(map::Region { keys });
        }
    }

    /// The message picked in the list, by its place in the map.
    pub(super) fn listed_on_map(&self) -> Option<usize> {
        let l = self.current()?;
        let ident = &self.accounts.get(l.account)?.ident;
        let index = self.index.lock().unwrap_or_else(|e| e.into_inner());
        // On Gmail the map reads All Mail, not the Inbox listed: there the
        // message is found by its Message-ID.
        let key = index.key_at(ident, &l.folder, l.summary.uid).or_else(|| {
            let id = &l.summary.message_id;
            index
                .records
                .values()
                .find(|r| r.key.account == *ident && !id.is_empty() && r.message_id == *id)
                .map(|r| r.key.clone())
        })?;
        drop(index);
        self.map.snapshot.as_ref()?.find(&key)
    }

    /// File the message picked in the list as the map does: `m` drops down
    /// the labels it may go to, `N` names a new one, `y` files it where it
    /// would go, and `-` says it is not one for there.
    pub(super) fn list_file(&mut self, act: Act) {
        let Some(i) = self.listed_on_map() else {
            self.status = if self.map_off.is_some() || self.map_later {
                "filing is by the map: M or M-g begins it".to_string()
            } else {
                "not on the map yet: it is filed once the map has read it".to_string()
            };
            return;
        };
        let Some(key) = self.map.snapshot.as_ref().map(|s| s.dots[i].key.clone()) else {
            return;
        };
        self.map.cursor = Some(key.clone());
        let suggested = self.picked_suggestion();
        match act {
            Act::Move => self.open_menu(i),
            Act::NewLabel => {
                self.map.moving = Some(vec![key]);
                self.ask_new_label();
            }
            Act::FileFirst if suggested.is_some() => {
                self.file_where_suggested(false);
            }
            Act::NotIt if suggested.is_some() => self.not_it(),
            _ => self.status = "nowhere it would go: m picks where".to_string(),
        }
    }

    /// The suggestions for the label last filed under, nearest first.
    pub fn suggestions(&self) -> Vec<(usize, f32)> {
        let (Some(s), Some(label)) = (&self.map.snapshot, &self.map.suggesting) else {
            return Vec::new();
        };
        s.model.suggestions(label, &s.not_it)
    }

    /// The message picked, if it is filed under no label yet, and the label
    /// it would go to.
    pub(super) fn picked_suggestion(&self) -> Option<(usize, String)> {
        let s = self.map.snapshot.as_ref()?;
        let i = self.map.picked()?;
        Some((i, s.suggestion(i)?.to_string()))
    }

    /// File the message picked, not yet filed, where it would go; or, all,
    /// mark every message shown that would go there too, and, marked, file
    /// them.
    pub(super) fn file_where_suggested(&mut self, all: bool) -> bool {
        let (Some(s), Some((i, label))) = (self.map.snapshot.clone(), self.picked_suggestion())
        else {
            return false;
        };
        if !all {
            self.file_keys(vec![s.dots[i].key.clone()], &label);
            return true;
        }
        let now = jiff::Timestamp::now().as_second();
        let keys: Vec<crate::index::Key> = self
            .map
            .shown(now)
            .into_iter()
            .filter(|&j| s.suggestion(j) == Some(label.as_str()))
            .map(|j| s.dots[j].key.clone())
            .collect();
        if self.map.region.as_ref().is_some_and(|r| r.keys == keys) {
            self.file_keys(keys, &label);
        } else {
            self.status = format!(
                "{} would go to {label}, marked: Y files them, esc lets go",
                keys.len()
            );
            self.map.region = Some(map::Region { keys });
        }
        true
    }

    /// File the first of the suggestions too, or all of them: those for the
    /// message picked, if it is filed under no label yet, else those for the
    /// label last filed under.
    pub(super) fn file_suggested(&mut self, all: bool) {
        if self.file_where_suggested(all) {
            return;
        }
        let (Some(s), Some(label)) = (self.map.snapshot.clone(), self.map.suggesting.clone())
        else {
            self.status =
                "nothing suggested: m files a message, and its like are suggested".to_string();
            return;
        };
        let keys: Vec<crate::index::Key> = self
            .suggestions()
            .into_iter()
            .take(if all { usize::MAX } else { 1 })
            .map(|(i, _)| s.dots[i].key.clone())
            .collect();
        if keys.is_empty() {
            self.status = format!("nothing more would go to {label}");
            return;
        }
        self.file_keys(keys, &label);
        if all {
            self.map.suggesting = None;
        }
    }

    /// Say the first suggestion is not one for its label: it is suggested
    /// no more, and pushed away from it.
    pub(super) fn not_it(&mut self) {
        let Some(s) = self.map.snapshot.clone() else {
            return;
        };
        let (i, label) = match (self.picked_suggestion(), self.map.suggesting.clone()) {
            (Some(picked), _) => picked,
            (None, Some(label)) => match self.suggestions().first() {
                Some(&(i, _)) => (i, label),
                None => return,
            },
            (None, None) => return,
        };
        let kept = self
            .index
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .keep(vec![crate::index::Kept::NotIt {
                key: s.dots[i].key.clone(),
                label: label.clone(),
            }]);
        self.status = match kept {
            Ok(()) => format!("not {label}: {} is suggested no more", s.dots[i].subject),
            Err(e) => format!("cannot keep that: {e:#}"),
        };
    }

    /// Ask what to call the label of the message picked: its cluster's, or
    /// the last it was given.
    pub(super) fn ask_rename_label(&mut self) {
        let Some(s) = self.map.snapshot.clone() else {
            return;
        };
        let label = self.map.picked().and_then(|i| {
            s.group_of(i)
                .and_then(|g| g.label.clone())
                .or_else(|| s.dots[i].labels.last().cloned())
        });
        match label {
            Some(l) => self.dialog = Dialog::RenameLabel(l.clone(), l),
            None => self.status = "the message picked has no label to rename".to_string(),
        }
    }

    /// Call the label `old` `new`, on every account that has it, and where
    /// letterbox alone keeps it; onto a label there is, the two are one.
    pub(super) fn rename_label(&mut self, old: &str, new: &str) {
        if new.is_empty() || crate::index::fold(old) == crate::index::fold(new) && old == new {
            self.status = format!("kept {old}");
            return;
        }
        let Some(s) = self.map.snapshot.clone() else {
            return;
        };
        let folded = crate::index::fold(old);
        let accounts: std::collections::BTreeSet<usize> = s
            .dots
            .iter()
            .filter(|d| d.labels.iter().any(|l| crate::index::fold(l) == folded))
            .filter_map(|d| self.place_of(d).map(|p| p.0))
            .collect();
        for a in accounts {
            self.ask(
                a,
                Ask::RenameLabel {
                    old: old.to_string(),
                    new: new.to_string(),
                },
            );
        }
        let kept = self
            .index
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .keep(vec![crate::index::Kept::Renamed {
                from: old.to_string(),
                to: new.to_string(),
            }]);
        if let Err(e) = kept {
            self.status = format!("cannot rename {old}: {e:#}");
            return;
        }
        if self.map.suggesting.as_deref().map(crate::index::fold) == Some(folded) {
            self.map.suggesting = Some(new.to_string());
        }
        self.status = format!("renamed {old} to {new}");
    }

    /// Take what a server said of filing a message: a label it does not
    /// keep, letterbox keeps; one it would not take is no longer pending.
    pub(super) fn filed(
        &mut self,
        a: usize,
        key: crate::index::Key,
        label: String,
        filed: Result<crate::store::Filed, String>,
    ) {
        let mut kept = Vec::new();
        match filed {
            Ok(f) if f.on_server => {}
            Ok(_) => {
                kept.push(crate::index::Kept::Label {
                    key: key.clone(),
                    label: label.clone(),
                    on: true,
                });
                kept.push(crate::index::Kept::Settled { key, label });
            }
            Err(e) => {
                self.status = format!("cannot file it under {label}: {}", first_line(&e));
                kept.push(crate::index::Kept::Settled { key, label });
            }
        }
        if !kept.is_empty() {
            let _ = self
                .index
                .lock()
                .unwrap_or_else(|e| e.into_inner())
                .keep(kept);
        }
        self.reload(a);
        self.map_pass(a);
    }
}
