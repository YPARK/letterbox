//! What is marked on the map, acted on together, moved onto a cluster;
//! and the map laid out again, to keep or drop.
use super::*;

impl App {
    /// Mark the messages `keys`, and say what is done with them.
    pub(super) fn mark(&mut self, keys: Vec<crate::index::Key>) {
        if keys.is_empty() {
            self.map.region = None;
            self.status = "nothing marked".to_string();
            return;
        }
        self.status = format!(
            "{} marked: d trashes them, e archives them, u and ! mark them, esc lets them go",
            keys.len()
        );
        self.map.region = Some(map::Region { keys });
    }

    /// The messages marked on the map.
    pub(super) fn region_dots(&self) -> Vec<map::Dot> {
        let (Some(s), Some(r)) = (&self.map.snapshot, &self.map.region) else {
            return Vec::new();
        };
        r.keys
            .iter()
            .filter_map(|k| s.find(k).map(|i| s.dots[i].clone()))
            .collect()
    }

    /// Mark every message marked on the map as `flag` says: all of them,
    /// unless all of them are, and then none.
    pub(super) fn region_flag(&mut self, flag: Flag) -> bool {
        let dots = self.region_dots();
        let has = |d: &map::Dot| match flag {
            Flag::Seen => d.flags.seen,
            Flag::Flagged => d.flags.flagged,
            Flag::Answered => d.flags.answered,
        };
        let on = !dots.iter().all(has);
        for d in &dots {
            if has(d) == on {
                continue;
            }
            let Some((a, folder, uid)) = self.place_of(d) else {
                continue;
            };
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
        let what = flag.said(on);
        self.status = format!("{} marked {what}", dots.len());
        true
    }

    /// Move every message marked on the map to its account's Trash; each
    /// `teach` takes learnt as thrown away, as it was chosen alone.
    pub(super) fn region_trash(&mut self, teach: &dyn Fn(&crate::index::Key) -> bool) -> bool {
        let dots = self.region_dots();
        let mut moved = 0;
        for d in &dots {
            let Some(listed) = self.listed_dot(d) else {
                continue;
            };
            let Some(trash) = self.trash_of(listed.account) else {
                continue;
            };
            let s = listed.summary;
            if teach(&d.key) {
                self.move_to_trash(listed.account, listed.folder, s.uid, s.message_id, trash);
            } else {
                self.trash_quietly(listed.account, listed.folder, s.uid, s.message_id, trash);
            }
            moved += 1;
        }
        self.map.region = None;
        self.status = if moved < dots.len() {
            format!(
                "{moved} of {} moved to Trash: d on one alone says which folder is Trash",
                dots.len()
            )
        } else {
            format!("{moved} moved to Trash · C-x u puts them back")
        };
        true
    }

    /// Take every message marked on the map that is in the Inbox out of it,
    /// given no label; learnt as kept if `teach`, as each was chosen alone.
    pub(super) fn region_archive(&mut self, teach: bool) -> bool {
        let dots = self.region_dots();
        let mut archived = 0;
        // Archived, they are kept: learnt as not thrown away.
        let keys: Vec<crate::index::Key> = dots
            .iter()
            .filter(|d| d.system.inbox)
            .map(|d| d.key.clone())
            .collect();
        if teach {
            self.teach(&keys, false);
        }
        for d in dots.iter().filter(|d| d.system.inbox) {
            let Some((a, folder, uid)) = self.place_of(d) else {
                continue;
            };
            self.ask(
                a,
                Ask::Archive {
                    folder,
                    uid,
                    message_id: d.message_id.clone(),
                },
            );
            archived += 1;
        }
        self.map.region = None;
        self.status = format!("{archived} archived, out of the Inbox");
        true
    }

    /// Begin moving the message picked, or what is marked, onto a cluster
    /// or a label.
    pub(super) fn start_moving(&mut self) {
        let keys = match &self.map.region {
            Some(r) => r.keys.clone(),
            None => match self.dot() {
                Some(d) => vec![d.key],
                None => {
                    self.status = "no message picked: the arrows or hjkl pick one".to_string();
                    return;
                }
            },
        };
        // Onto a cluster: out to the clusters, where each is to be picked.
        self.map.close_tree();
        self.status = format!(
            "moving {}: ⏎ on a cluster, or a click on its anchor; N a new label; esc stops",
            if keys.len() == 1 {
                "it".to_string()
            } else {
                keys.len().to_string()
            }
        );
        self.map.moving = Some(keys);
    }

    /// Move what is moving onto the cluster `id`: under its label, or, if it
    /// is none yet, under one asked for, its first word given.
    pub(super) fn move_onto(&mut self, id: u32) {
        let Some(g) = self
            .map
            .snapshot
            .as_ref()
            .and_then(|s| s.group(id))
            .cloned()
        else {
            return;
        };
        match g.label {
            Some(label) => {
                let keys = self.map.moving.take().unwrap_or_default();
                self.file_keys(keys, &label);
            }
            None => {
                let word = g.words.first().cloned().unwrap_or_default();
                self.dialog = Dialog::Label(capitalised(&word));
            }
        }
    }

    /// Ask the name of a new label for what is moving, its commonest word
    /// given.
    pub(super) fn ask_new_label(&mut self) {
        let dots: Vec<map::Dot> = {
            let (Some(s), Some(keys)) = (&self.map.snapshot, &self.map.moving) else {
                return;
            };
            keys.iter()
                .filter_map(|k| s.find(k).map(|i| s.dots[i].clone()))
                .collect()
        };
        let mut counts: std::collections::BTreeMap<String, usize> = Default::default();
        for d in &dots {
            for w in crate::model::words(crate::model::bare_subject(&d.subject)) {
                *counts.entry(w).or_default() += 1;
            }
        }
        let word = counts
            .into_iter()
            .max_by(|a, b| a.1.cmp(&b.1).then(b.0.cmp(&a.0)))
            .map(|(w, _)| w)
            .unwrap_or_default();
        self.dialog = Dialog::Label(capitalised(&word));
    }

    /// Lay the map out again, off this thread, from where each message is,
    /// to be shown before it is kept.
    pub(super) fn relayout(&mut self) {
        let Some(s) = self.map.snapshot.clone() else {
            return;
        };
        if s.at.is_none() || self.map.relaying {
            return;
        }
        self.map.relaying = true;
        self.pending += 1;
        self.status = "laying the map out again…".to_string();
        let replies = self.replies_to.clone();
        std::thread::spawn(move || {
            let at = map::relayout(&s);
            let _ = replies.send(Reply {
                account: usize::MAX,
                answer: Answer::Relaid {
                    lens: s.lens,
                    keys: s.model.keys.clone(),
                    at,
                },
            });
        });
    }

    /// Show the map laid out again, onto the map as it is now: mail come
    /// while it was laid out stays where it is, mail gone is let go.
    pub(super) fn relaid(
        &mut self,
        lens: crate::index::Lens,
        keys: &[crate::index::Key],
        at: &[crate::model::Point],
    ) {
        self.map.relaying = false;
        let Some(s) = self.map.snapshot.clone() else {
            return;
        };
        let at = match map::onto(&s, keys, at) {
            Some(at) if s.lens == lens => at,
            _ => {
                self.status =
                    "the map was turned while it was laid out again: M-r again".to_string();
                return;
            }
        };
        let moved = map::moved(&s, &at);
        let faith = crate::model::faithfulness(&s.model, &at);
        // No nearer than it was: said, and M-r not offered again for it.
        let was = s.faithfulness.unwrap_or(0.0);
        let nearer = faith > was + map::NEARER;
        if !nearer {
            self.map.tried = Some(was);
        }
        self.status = format!(
            "laid out again, {moved} moved, {:.0}% faithful{}: ⏎ keeps it, esc drops it",
            faith * 100.0,
            if nearer { "" } else { ", no nearer" }
        );
        self.map.preview = Some(at);
    }

    /// Keep the map as it was laid out again.
    pub(super) fn keep_relayout(&mut self) {
        let (Some(s), Some(at)) = (self.map.snapshot.clone(), self.map.preview.take()) else {
            return;
        };
        // Laid out again, the clusters' blocks are tiled anew.
        self.map.mosaic = None;
        let layout = s
            .model
            .keys
            .iter()
            .zip(&at)
            .map(|(k, p)| (k.clone(), p[0], p[1]))
            .collect();
        let kept = self
            .index
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .keep(vec![crate::index::Kept::Layout {
                lens: s.lens,
                at: layout,
            }]);
        self.status = match kept {
            Ok(()) => "the map, laid out again, kept".to_string(),
            Err(e) => format!("the map cannot keep its layout: {e:#}"),
        };
    }
}
