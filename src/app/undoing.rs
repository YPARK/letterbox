//! C-x u for what was last moved, however it was: archived, filed, moved
//! onto a cluster, trashed alone, thrown away as junk, or what the Inbox
//! looked through did. Each message is remembered as it was before it was
//! asked to go: where it was, the labels its server kept on it, and those
//! letterbox alone kept. Put back, it is made so again, from where its
//! server said it landed, or else from where the map's next look finds it;
//! and what moving it taught is taken back.

use super::*;
use crate::index::{Kept, Key};

/// A message as it was before it was asked to go.
#[derive(Debug)]
pub(super) struct Before {
    a: usize,
    key: Key,
    message_id: String,
    /// Where it was asked to go from.
    folder: String,
    uid: u32,
    /// Where it was, by folder and UID; its server's labels; and those
    /// letterbox alone kept.
    places: Vec<(String, u32)>,
    server: Vec<String>,
    kept: Vec<String>,
    /// Its server answered that it went.
    answered: bool,
    /// Where it landed, as its server said: a move's, or a filing's.
    landed: Option<(String, u32)>,
    /// The map's looks at its account since it was answered.
    passes: u8,
}

/// What was last moved together, for C-x u: when the key that moved it
/// was pressed, each as it was, and how many letterbox did not know.
#[derive(Debug, Default)]
pub struct Moves {
    at: i64,
    before: Vec<Before>,
    unknown: usize,
}

/// The message an ask moves, if it moves one: the folder and UID it is
/// moved from, its `Message-ID`, and its key, if the ask says.
fn moved_from(ask: &Ask) -> Option<(&str, u32, &str, Option<&Key>)> {
    match ask {
        Ask::Move {
            from,
            uid,
            message_id,
            ..
        }
        | Ask::Archive {
            folder: from,
            uid,
            message_id,
        } => Some((from, *uid, message_id, None)),
        Ask::File {
            folder,
            uid,
            message_id,
            key,
            ..
        } => Some((folder, *uid, message_id, Some(key))),
        _ => None,
    }
}

/// Gmail's labels on a message, as they are put back: its own, and the
/// Inbox's, by its place there or its `\Inbox`; not Gmail's other marks.
fn gmail_labels(places: &[(String, u32)], server: &[String], inbox: &str) -> Vec<String> {
    let mut labels: Vec<String> = server
        .iter()
        .filter(|l| !l.starts_with('\\') || l.as_str() == "\\Inbox")
        .cloned()
        .collect();
    if places.iter().any(|(f, _)| f == inbox) && !labels.iter().any(|l| l == "\\Inbox") {
        labels.push("\\Inbox".to_string());
    }
    labels
}

/// Those of `a` not in `b`.
fn less(a: &[String], b: &[String]) -> Vec<String> {
    a.iter().filter(|x| !b.contains(x)).cloned().collect()
}

impl App {
    /// Before `ask` goes, remember the message it moves as it is, for C-x
    /// u: the first such of a key pressed begins what is remembered anew.
    pub(super) fn remember_move(&mut self, account: usize, ask: &Ask) {
        if self.quiet_moves {
            return;
        }
        let Some((folder, uid, message_id, key)) = moved_from(ask) else {
            return;
        };
        if !std::mem::replace(&mut self.moves_begun, true) {
            self.undoable = Moves {
                at: self.input_at,
                ..Moves::default()
            };
            self.last = Some(Last::Moves);
        }
        let ident = self.accounts[account].ident.clone();
        let index = self.index.lock().unwrap_or_else(|e| e.into_inner());
        // By its key, else where it is, else its Message-ID: Gmail's are
        // known by where they are in All Mail, not the Inbox.
        let record = key
            .and_then(|k| index.records.get(k))
            .or_else(|| {
                index
                    .key_at(&ident, folder, uid)
                    .and_then(|k| index.records.get(&k))
            })
            .or_else(|| {
                (!message_id.is_empty()).then_some(())?;
                index
                    .records
                    .values()
                    .find(|r| r.key.account == ident && r.message_id == message_id)
            });
        let Some(record) = record else {
            drop(index);
            self.undoable.unknown += 1;
            return;
        };
        let before = Before {
            a: account,
            key: record.key.clone(),
            message_id: record.message_id.clone(),
            folder: folder.to_string(),
            uid,
            places: record
                .places
                .iter()
                .map(|p| (p.folder.clone(), p.uid))
                .collect(),
            server: record.server.clone(),
            kept: index
                .keep
                .labels
                .get(&record.key)
                .cloned()
                .unwrap_or_default(),
            answered: false,
            landed: None,
            passes: 0,
        };
        drop(index);
        self.undoable.before.push(before);
    }

    /// A move of what is remembered answered: where it landed, if its
    /// server said; not moved, it is no longer to be put back.
    pub(super) fn move_answered(
        &mut self,
        a: usize,
        folder: &str,
        uid: u32,
        landed: Result<Option<(String, u32)>, ()>,
    ) {
        let Some(at) = self
            .undoable
            .before
            .iter()
            .position(|b| b.a == a && b.folder == folder && b.uid == uid && !b.answered)
        else {
            return;
        };
        match landed {
            Ok(landed) => {
                let b = &mut self.undoable.before[at];
                b.answered = true;
                b.landed = landed;
            }
            Err(()) => {
                self.undoable.before.remove(at);
            }
        }
    }

    /// The map looked at the account `a` again.
    pub(super) fn moves_looked(&mut self, a: usize) {
        for b in self
            .undoable
            .before
            .iter_mut()
            .filter(|b| b.a == a && b.answered)
        {
            b.passes = b.passes.saturating_add(1);
        }
    }

    /// C-x u: put what was last moved back as it was, each that its server
    /// has answered and the map has looked at since; the rest by C-x u
    /// again.
    pub(super) fn undo_moves(&mut self) {
        let moves = std::mem::take(&mut self.undoable);
        if moves.before.is_empty() {
            self.status = match moves.unknown {
                0 => "nothing moved to put back".to_string(),
                n => {
                    format!("{n} moved, but letterbox could not tell where they were, so are left")
                }
            };
            return;
        }
        let mut asks: Vec<(usize, Ask)> = Vec::new();
        let mut kept: Vec<Kept> = Vec::new();
        let mut done: Vec<Key> = Vec::new();
        let mut waiting = Vec::new();
        let mut gone = 0;
        {
            let index = self.index.lock().unwrap_or_else(|e| e.into_inner());
            for b in moves.before {
                let record = index.records.get(&b.key);
                let now_kept = index.keep.labels.get(&b.key).cloned().unwrap_or_default();
                let changed = record.is_some_and(|r| {
                    let places: Vec<(String, u32)> =
                        r.places.iter().map(|p| (p.folder.clone(), p.uid)).collect();
                    places != b.places || r.server != b.server
                }) || now_kept != b.kept;
                let ready = b.answered && (b.landed.is_some() || changed || b.passes >= 2);
                if !ready {
                    waiting.push(b);
                    continue;
                }
                for l in less(&b.kept, &now_kept) {
                    kept.push(Kept::Label {
                        key: b.key.clone(),
                        label: l,
                        on: true,
                    });
                }
                for l in less(&now_kept, &b.kept) {
                    kept.push(Kept::Label {
                        key: b.key.clone(),
                        label: l,
                        on: false,
                    });
                }
                let back = self.put_back(&b, record);
                if back.is_empty() && b.landed.is_none() && record.is_none() {
                    gone += 1;
                    continue;
                }
                asks.extend(back.into_iter().map(|ask| (b.a, ask)));
                done.push(b.key.clone());
            }
            drop(index);
        }
        if !done.is_empty() {
            let at = moves.at;
            let forgotten = self
                .index
                .lock()
                .unwrap_or_else(|e| e.into_inner())
                .forget_where(|t| !t.auto && t.at >= at && done.contains(&t.key));
            if let Err(e) = forgotten {
                self.status = format!("cannot take back what was learnt: {e:#}");
            }
        }
        if !kept.is_empty() {
            let _ = self
                .index
                .lock()
                .unwrap_or_else(|e| e.into_inner())
                .keep(kept);
        }
        self.quiet_moves = true;
        for (a, ask) in asks {
            self.ask(a, ask);
        }
        self.quiet_moves = false;
        let mut said = match done.len() {
            1 => "1 put back as it was".to_string(),
            n => format!("{n} put back as they were"),
        };
        if !waiting.is_empty() {
            said.push_str(&format!(
                " · {} still being moved: C-x u again once they are",
                waiting.len()
            ));
        }
        let unknown = moves.unknown + gone;
        if unknown > 0 {
            said.push_str(&format!(
                " · {unknown} letterbox could not tell where they were, so are left where they went"
            ));
        }
        self.status = said;
        if !waiting.is_empty() {
            self.undoable = Moves {
                at: moves.at,
                before: waiting,
                unknown: 0,
            };
            self.last = Some(Last::Moves);
        }
        for a in self.live().collect::<Vec<_>>() {
            self.reload(a);
        }
    }

    /// What puts the message `b` back as it was, `record` as the map now
    /// has it: its labels given back or taken off where it is, then moved
    /// back to where it was.
    fn put_back(&self, b: &Before, record: Option<&crate::index::Record>) -> Vec<Ask> {
        let gmail = b.key.id.starts_with("gm:");
        let inbox = self.inbox(b.a);
        let now: Vec<(String, u32)> = record
            .map(|r| r.places.iter().map(|p| (p.folder.clone(), p.uid)).collect())
            .unwrap_or_default();
        let server_now: Vec<String> = record.map(|r| r.server.clone()).unwrap_or_default();
        // Where it is: where its server said it landed, else where the map
        // last found it, other than where it was.
        let here = b.landed.clone().or_else(|| {
            now.iter()
                .find(|(f, _)| !b.places.iter().any(|(g, _)| g == f))
                .or_else(|| now.first())
                .cloned()
        });
        let Some((folder, uid)) = here else {
            return Vec::new();
        };
        let relabel = |on: Option<String>, off: Option<String>| Ask::Relabel {
            folder: folder.clone(),
            uid,
            off,
            on,
        };
        let mut asks = Vec::new();
        if gmail && b.landed.is_none() {
            // On Gmail, where a message is is its labels: given back first,
            // then taken off, so that it is never without them all.
            let was = gmail_labels(&b.places, &b.server, &inbox);
            let is = gmail_labels(&now, &server_now, &inbox);
            asks.extend(less(&was, &is).into_iter().map(|l| relabel(Some(l), None)));
            asks.extend(less(&is, &was).into_iter().map(|l| relabel(None, Some(l))));
            return asks;
        }
        if !gmail && record.is_some() {
            // A server's keywords, changed where it is now, before it moves:
            // known only while the map knows it, as it does not in Trash.
            let unkey = |ls: &[String]| -> Vec<String> {
                ls.iter()
                    .filter(|l| !l.starts_with('\\'))
                    .map(|l| crate::index::labels::unkeyword(l))
                    .collect()
            };
            let (was, is) = (unkey(&b.server), unkey(&server_now));
            asks.extend(less(&was, &is).into_iter().map(|l| relabel(Some(l), None)));
            asks.extend(less(&is, &was).into_iter().map(|l| relabel(None, Some(l))));
        }
        let gone_from_where_it_was = !b.places.iter().any(|(f, _)| *f == folder);
        if gone_from_where_it_was {
            asks.push(Ask::Move {
                from: folder.clone(),
                uid,
                to: b.folder.clone(),
                message_id: b.message_id.clone(),
                trashed: false,
            });
        }
        asks
    }
}
