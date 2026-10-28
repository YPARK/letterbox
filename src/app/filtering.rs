//! What letterbox is sure is junk, as it has learnt: asked about, never
//! thrown away unless you choose it; and what was thrown away, told of to
//! be put back.
use super::*;

impl App {
    /// Ask about each message come to an Inbox since it was turned on that
    /// it is sure enough of, and that you never decided on: nothing is
    /// thrown away unless you choose it. Each is asked about once.
    pub fn throw_away_on_its_own(&mut self) {
        if !self.auto.on {
            return;
        }
        let found: Vec<Junk> = {
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
                .filter(|r| !self.junk_asked.contains(&r.key))
                .filter_map(|r| {
                    let tokens = crate::filter::Filter::tokens(r, &known);
                    let odds = index.filter.odds(&tokens);
                    (odds >= crate::filter::AUTO).then(|| Junk {
                        key: r.key.clone(),
                        from: r.from.clone(),
                        subject: r.subject.clone(),
                        odds,
                        why: index.filter.why(&tokens),
                        chosen: false,
                    })
                })
                .collect()
        };
        if found.is_empty() {
            return;
        }
        self.junk_asked.extend(found.iter().map(|j| j.key.clone()));
        // Said, never a pop-up of its own: a key pressed as it came would
        // act on it.
        let n = found.len();
        self.junk.extend(found);
        if self.compose.is_none() {
            self.status =
                format!("{n} new look like junk: J shows them, i looks through the Inbox");
        }
    }

    /// How many look like junk, held for J.
    pub fn junk_held(&self) -> usize {
        self.junk.len()
    }

    /// J: what looked like junk and is still in an Inbox, unread, to
    /// choose what to do with.
    pub(super) fn show_junk(&mut self) {
        {
            let index = self.index.lock().unwrap_or_else(|e| e.into_inner());
            self.junk.retain(|j| {
                index.records.get(&j.key).is_some_and(|r| !r.flags.seen)
                    && index.system(&j.key).inbox
            });
        }
        if self.junk.is_empty() {
            self.status = "nothing new looks like junk".to_string();
            return;
        }
        let items = self
            .junk
            .iter()
            .cloned()
            .map(|j| Junk { chosen: false, ..j })
            .collect();
        self.dialog = Dialog::Junk(JunkPick { items, at: 0 });
    }

    /// A key in the list of what looks like junk: ↑ ↓ go along it, space
    /// chooses the one picked, or lets it go, and goes on; d throws away
    /// those chosen, and nothing if none is, never learnt, as the filter's
    /// own guess; n says the one picked is not junk, learnt as kept; i looks through
    /// the Inbox; any other key leaves them as they are.
    pub(super) fn junk_key(&mut self, key: KeyEvent) {
        let Dialog::Junk(pick) = &mut self.dialog else {
            return;
        };
        let plain = key.modifiers.is_empty() || key.modifiers == KeyModifiers::SHIFT;
        let last = pick.items.len().saturating_sub(1);
        match (keymap::act(key), key.code) {
            (Some(Act::Down), _) => pick.at = (pick.at + 1).min(last),
            (Some(Act::Up), _) => pick.at = pick.at.saturating_sub(1),
            (_, KeyCode::Char(' ')) if plain => {
                if let Some(j) = pick.items.get_mut(pick.at) {
                    j.chosen = !j.chosen;
                }
                pick.at = (pick.at + 1).min(last);
            }
            (_, KeyCode::Char('d')) if plain => {
                let chosen: Vec<Junk> = pick.items.iter().filter(|j| j.chosen).cloned().collect();
                if chosen.is_empty() {
                    self.status = "nothing chosen: space chooses the one picked".to_string();
                    return;
                }
                self.dialog = Dialog::Closed;
                self.throw_held(chosen);
            }
            // Not junk, as you say: learnt as kept, your own decision, and
            // asked of no more.
            (_, KeyCode::Char('n')) if plain => {
                let Some(j) = pick.items.get(pick.at).cloned() else {
                    return;
                };
                pick.items.remove(pick.at);
                pick.at = pick.at.min(pick.items.len().saturating_sub(1));
                if pick.items.is_empty() {
                    self.dialog = Dialog::Closed;
                }
                self.junk.retain(|k| k.key != j.key);
                self.teach(std::slice::from_ref(&j.key), false);
                self.status = format!("{} kept, and learnt so: not junk", j.subject);
            }
            (_, KeyCode::Char('i')) if plain => {
                self.dialog = Dialog::Closed;
                self.open_review();
            }
            _ => {
                self.dialog = Dialog::Closed;
                self.status =
                    "left as they are · J again, or i looks through the Inbox".to_string();
            }
        }
    }

    /// Throw away those `chosen`, as you chose: written down to be told of
    /// by B and put back, never learnt. Each thrown is held no more; each
    /// that could not be is said, and held still.
    fn throw_held(&mut self, chosen: Vec<Junk>) {
        let mut thrown = 0;
        let mut not: Vec<String> = Vec::new();
        for j in chosen {
            let found = self
                .index
                .lock()
                .unwrap_or_else(|e| e.into_inner())
                .records
                .get(&j.key)
                .map(|r| (r.places.clone(), r.message_id.clone()));
            let Some((places, message_id)) = found else {
                not.push(format!("{}: not known any more", j.subject));
                continue;
            };
            let Some(a) = self
                .live()
                .find(|&a| self.accounts[a].ident == j.key.account)
            else {
                not.push(format!("{}: its account is not open", j.subject));
                continue;
            };
            let (Some(trash), inbox) = (self.trash_of(a), self.inbox(a)) else {
                not.push(format!(
                    "{}: no Trash known · d on it alone asks which",
                    j.subject
                ));
                continue;
            };
            let Some(place) = places.iter().find(|p| p.folder == inbox) else {
                not.push(format!("{}: in the Inbox no more", j.subject));
                continue;
            };
            let written = self
                .index
                .lock()
                .unwrap_or_else(|e| e.into_inner())
                .throw_on_its_own(&j.key);
            if let Err(e) = written {
                not.push(format!("{}: {e:#}", j.subject));
                continue;
            }
            self.trash_quietly(a, place.folder.clone(), place.uid, message_id, trash);
            self.junk.retain(|k| k.key != j.key);
            thrown += 1;
        }
        self.status = match not.first() {
            None => format!("{thrown} thrown away: B says which, and why, to put back"),
            Some(first) => format!(
                "{thrown} thrown away · {} not: {first}{}",
                not.len(),
                if not.len() > 1 { ", …" } else { "" }
            ),
        };
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
            self.status = "nothing thrown away as junk in the last 7 days".to_string();
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

/// A message come to an Inbox that letterbox is sure is junk, held to be
/// asked about; and, listed by J, whether it is chosen to be thrown away.
#[derive(Clone, Debug)]
pub struct Junk {
    pub key: crate::index::Key,
    pub from: String,
    pub subject: String,
    pub odds: f32,
    /// What weighed most toward it, in words: for you to say whether it is.
    pub why: Vec<String>,
    pub chosen: bool,
}

/// What looks like junk, listed by J: the one picked, and each chosen.
#[derive(Clone, Debug)]
pub struct JunkPick {
    pub items: Vec<Junk>,
    pub at: usize,
}
