//! What letterbox throws away on its own, as it has learnt, and what it
//! threw away, told of to be put back.
use super::*;

impl App {
    /// Throw away, on its own, each message come to an Inbox since it was
    /// turned on that it is sure enough of, and that you never decided on:
    /// written down to be told of, never learnt.
    pub fn throw_away_on_its_own(&mut self) {
        if !self.auto.on {
            return;
        }
        let found: Vec<(crate::index::Key, Vec<crate::index::Place>, String)> = {
            let index = self.index.lock().unwrap_or_else(|e| e.into_inner());
            if !index.filter.ready() {
                return;
            }
            let known = index.known();
            let decided: std::collections::HashSet<&crate::index::Key> =
                index.filter.taught.iter().map(|t| &t.key).collect();
            index
                .records
                .values()
                // New, and never read: mail read and left is yours.
                .filter(|r| r.first_seen >= self.auto.since && !decided.contains(&r.key))
                .filter(|r| !r.flags.seen)
                .filter(|r| index.system(&r.key).inbox)
                .filter(|r| {
                    let tokens = crate::filter::Filter::tokens(r, &known);
                    index.filter.odds(&tokens) >= crate::filter::AUTO
                })
                .map(|r| (r.key.clone(), r.places.clone(), r.message_id.clone()))
                .collect()
        };
        let mut thrown = 0;
        for (key, places, message_id) in found {
            let Some(a) = self.live().find(|&a| self.accounts[a].ident == key.account) else {
                continue;
            };
            let (Some(trash), inbox) = (self.trash_of(a), self.inbox(a)) else {
                continue;
            };
            let Some(place) = places.iter().find(|p| p.folder == inbox) else {
                continue;
            };
            let written = self
                .index
                .lock()
                .unwrap_or_else(|e| e.into_inner())
                .throw_on_its_own(&key);
            if written.is_err() {
                continue;
            }
            // Done on its own: not followed, nor said in a pop-up.
            self.quiet = true;
            self.trash_quietly(a, place.folder.clone(), place.uid, message_id, trash);
            self.quiet = false;
            thrown += 1;
        }
        if thrown > 0 {
            self.status = format!("{thrown} thrown away on its own: B says which, and why");
        }
    }

    /// As letterbox opens: tell of what was thrown away on its own since
    /// it was last told of, if anything was.
    pub fn debrief_on_start(&mut self) {
        let since = self.auto.debriefed;
        self.debrief_since(since, "Since you last looked");
    }

    /// Tell of what was thrown away on its own after `since`, under
    /// `title`; whether there was any.
    pub(super) fn debrief_since(&mut self, since: i64, title: &str) -> bool {
        let mut d = {
            let index = self.index.lock().unwrap_or_else(|e| e.into_inner());
            let d = index.debrief(since);
            if d.thrown.is_empty() {
                return false;
            }
            let said = crate::filter::Filter::check(&index.filter.taught);
            crate::filter::Debrief {
                grade: (!said.is_empty()).then(|| crate::filter::grade(&said, crate::filter::AUTO)),
                ..d
            }
        };
        d.title = title.to_string();
        self.dialog = Dialog::Debrief(Box::new(d));
        self.auto.debriefed = jiff::Timestamp::now().as_second();
        self.remember_auto();
        true
    }

    /// Tell of what was thrown away on its own in the days it can be put
    /// back.
    pub(super) fn debrief(&mut self) {
        let since = jiff::Timestamp::now().as_second() - crate::trash::KEPT;
        if !self.debrief_since(since, "Thrown away on its own, the last 7 days") {
            self.status = "nothing thrown away on its own in the last 7 days".to_string();
        }
    }

    /// A key in the debrief: ⏎ puts the one picked back, u all of them.
    pub(super) fn debrief_key(&mut self, key: KeyEvent) {
        let Dialog::Debrief(d) = &mut self.dialog else {
            return;
        };
        let n = d.thrown.len();
        match (keymap::act(key), key.code) {
            (Some(Act::Down), _) => d.at = (d.at + 1).min(n.saturating_sub(1)),
            (Some(Act::Up), _) => d.at = d.at.saturating_sub(1),
            (Some(Act::Open), _) => {
                let at = d.at;
                self.put_back_thrown(&[at]);
            }
            (_, KeyCode::Char('u')) => self.put_back_thrown(&(0..n).collect::<Vec<_>>()),
            _ => self.dialog = Dialog::Closed,
        }
    }

    /// Put back those `at` of what was thrown away on its own, each learnt
    /// as kept.
    pub(super) fn put_back_thrown(&mut self, at: &[usize]) {
        let Dialog::Debrief(mut d) = std::mem::take(&mut self.dialog) else {
            return;
        };
        let mut back = 0;
        for &i in at {
            let Some(t) = d.thrown.get(i).filter(|t| !t.back).cloned() else {
                continue;
            };
            let Some(a) = self
                .live()
                .find(|&a| self.accounts[a].ident == t.key.account)
            else {
                continue;
            };
            let email = self.accounts[a].email.clone();
            let Some(e) = self.ledger.find(&email, &t.message_id) else {
                continue;
            };
            let _ = self
                .index
                .lock()
                .unwrap_or_else(|e| e.into_inner())
                .put_back(&t.key.account, &t.message_id);
            self.ask(
                a,
                Ask::Move {
                    from: e.folder,
                    uid: e.uid,
                    to: e.origin,
                    message_id: t.message_id.clone(),
                    trashed: false,
                },
            );
            d.thrown[i].back = true;
            back += 1;
        }
        self.status = match back {
            0 => "nothing put back: not in the Trash yet, or put back already".to_string(),
            1 => "put back, and learnt as kept".to_string(),
            n => format!("{n} put back, and learnt as kept"),
        };
        self.dialog = Dialog::Debrief(d);
    }

    /// Write down whether new mail is thrown away on its own, and when what
    /// it did was last told of.
    pub fn remember_auto(&self) {
        if let Some(path) = &self.filter_file {
            let _ = std::fs::write(path, self.auto.to_string());
        }
    }
}
