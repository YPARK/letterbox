//! A label renamed from the map: by R, or a right-click on its anchor;
//! asked first, how many carry it and where; never onto a label there is,
//! as two made one cannot be told apart again; and renamed back by C-x u.
use super::*;

/// What may be done with a label, by a right-click on its anchor.
pub const LABEL_MENU: [&str; 3] = ["Rename…", "Split into subfolders…", "Remove…"];

/// What was done last that C-x u undoes.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Last {
    /// A label renamed: from, to.
    Rename(String, String),
    /// A label split into subfolders, or removed.
    Refile,
    /// Mail moved, however: archived, filed, trashed alone.
    Moves,
}

impl App {
    /// A right-click on a cluster's anchor, or R: what may be done with it.
    pub(super) fn cluster_menu(&mut self, id: u32) {
        let label = self
            .map
            .snapshot
            .as_ref()
            .and_then(|s| s.group(id))
            .and_then(|g| g.label.clone());
        if label.is_none() {
            self.status = "no label to rename: m files mail under one".to_string();
            return;
        }
        self.dialog = Dialog::ClusterMenu { id, at: 0 };
    }

    /// A key in a label's menu: ↑ ↓ pick, ⏎ does what is picked; any
    /// other key closes it.
    pub(super) fn cluster_menu_key(&mut self, key: KeyEvent) {
        let Dialog::ClusterMenu { id, at } = &mut self.dialog else {
            return;
        };
        let (id, last) = (*id, LABEL_MENU.len() - 1);
        match (keymap::act(key), key.code) {
            (Some(Act::Down), _) => *at = (*at + 1).min(last),
            (Some(Act::Up), _) => *at = at.saturating_sub(1),
            (_, KeyCode::Enter) => {
                let at = *at;
                self.dialog = Dialog::Closed;
                match at {
                    0 => self.ask_rename_of(id),
                    1 => self.ask_split(id),
                    _ => self.ask_remove(id),
                }
            }
            _ => self.dialog = Dialog::Closed,
        }
    }

    /// Ask what the label of the cluster `id` is to be called.
    pub(super) fn ask_rename_of(&mut self, id: u32) {
        let label = self
            .map
            .snapshot
            .as_ref()
            .and_then(|s| s.group(id))
            .and_then(|g| g.label.clone());
        match label {
            Some(l) => self.dialog = Dialog::RenameLabel(l.clone(), l),
            None => self.status = "no label to rename".to_string(),
        }
    }

    /// The label `old` to be called `new`, as typed: asked first, saying
    /// how many carry it and on which accounts; never onto a label there
    /// is.
    pub(super) fn confirm_rename(&mut self, old: &str, new: &str) {
        if new.is_empty() || new == old {
            self.status = format!("kept {old}");
            return;
        }
        let Some(s) = self.map.snapshot.clone() else {
            return;
        };
        let (was, will) = (crate::index::fold(old), crate::index::fold(new));
        let there = will != was
            && (s
                .groups
                .iter()
                .any(|g| g.label.as_deref().map(crate::index::fold) == Some(will.clone()))
                || s.dots
                    .iter()
                    .any(|d| d.labels.iter().any(|l| crate::index::fold(l) == will)));
        if there {
            self.status = format!(
                "{new} is a label already: two made one could not be told apart again, so nothing is renamed"
            );
            return;
        }
        let carrying: Vec<&map::Dot> = s
            .dots
            .iter()
            .filter(|d| d.labels.iter().any(|l| crate::index::fold(l) == was))
            .collect();
        let accounts: std::collections::BTreeSet<String> = carrying
            .iter()
            .filter_map(|d| self.place_of(d).map(|p| self.accounts[p.0].name.clone()))
            .collect();
        self.dialog = Dialog::ConfirmRename {
            old: old.to_string(),
            new: new.to_string(),
            count: carrying.len(),
            accounts: accounts.into_iter().collect(),
        };
    }

    /// A key in the pop-up asking to rename: y renames, any other key
    /// keeps it.
    pub(super) fn confirm_rename_key(&mut self, key: KeyEvent) {
        let Dialog::ConfirmRename { old, new, .. } = std::mem::take(&mut self.dialog) else {
            return;
        };
        if matches!(key.code, KeyCode::Char('y' | 'Y')) && key.modifiers.is_empty() {
            if self.rename_label(&old, &new) {
                self.status.push_str(" · C-x u renames it back");
                self.last = Some(Last::Rename(old, new));
            }
        } else {
            self.status = format!("kept {old}");
        }
    }

    /// C-x u: undo what was done last: mail moved, however, or a label
    /// renamed, split or removed.
    pub(super) fn undo_last(&mut self) {
        match self.last.take() {
            Some(Last::Rename(old, new)) => {
                if self.rename_label(&new, &old) {
                    self.status = format!("{new} renamed back to {old}");
                }
            }
            Some(Last::Refile) => self.undo_refiling(),
            Some(Last::Moves) | None => self.undo_moves(),
        }
    }
}
