//! Choosing in the Inbox looked through for many at once: for all those
//! suggested alike, or all selected with the mouse; and a choice taken
//! back before it is done.
use super::*;

impl App {
    /// Every message of the Inbox looked through suggested to go where the
    /// message `i` is, chosen to go there: nothing is done till C-c C-c,
    /// so nothing is asked. Chosen together, nothing is learnt of each.
    pub(super) fn choose_like(&mut self, i: usize) {
        let Some(s) = self.map.snapshot.clone() else {
            return;
        };
        let like = Self::suggested(&s, i);
        let all: Vec<usize> = self
            .review_rows()
            .into_iter()
            .filter(|&j| Self::suggested(&s, j) == like)
            .collect();
        for &j in &all {
            self.decide(j, Some(like.clone()), false);
            self.untaught.insert(s.dots[j].key.clone());
        }
        if let Some(r) = &mut self.review {
            r.join(all.len());
        }
        let to = match &like {
            Choice::Move(label) => format!("to go to {label}"),
            Choice::Discard => "to be discarded".to_string(),
            _ => "to be kept".to_string(),
        };
        self.status = format!("{} {to} · C-c C-c does it, with the rest", all.len());
        self.review_step(1);
    }

    /// Take back the last choice made in the Inbox looked through, and pick
    /// the message it was made for; with none, what was last done.
    pub(super) fn unchoose(&mut self) -> bool {
        let Some(last) = self.review.as_mut().and_then(|r| r.before.pop()) else {
            return false;
        };
        let n = last.len();
        let first = last.first().map(|(k, _)| k.clone());
        if let Some(r) = &mut self.review {
            for (key, was) in last.into_iter().rev() {
                match was {
                    Some(c) => r.chosen.insert(key, c),
                    None => r.chosen.remove(&key),
                };
            }
        }
        let i = first.and_then(|k| self.map.snapshot.as_ref()?.find(&k));
        if i.is_some() {
            self.pick_review(i);
        }
        self.status = match n {
            1 => "the choice taken back".to_string(),
            n => format!("the choice for {n} taken back"),
        };
        true
    }

    /// Select the rows of the Inbox looked through from `a` to `b`, both,
    /// the row `b` picked.
    pub(super) fn review_select(&mut self, a: usize, b: usize) {
        let rows = self.review_rows();
        let Some(s) = self.map.snapshot.clone() else {
            return;
        };
        let last = rows.len().saturating_sub(1);
        let (lo, hi) = (a.min(b).min(last), a.max(b).min(last));
        let Some(r) = &mut self.review else {
            return;
        };
        r.selected = rows
            .get(lo..=hi)
            .unwrap_or_default()
            .iter()
            .map(|&i| s.dots[i].key.clone())
            .collect();
        let n = r.selected.len();
        if let Some(&i) = rows.get(b.min(last)) {
            self.pick_review(Some(i));
        }
        self.focus = Focus::Reader;
        self.status = format!(
            "{n} selected: m moves them, d discards them, space keeps them, y as suggested; esc lets them go"
        );
    }

    /// Add the row `row` of the Inbox looked through to what is selected,
    /// or take it away.
    pub(super) fn review_toggle(&mut self, row: usize) {
        let Some(&i) = self.review_rows().get(row) else {
            return;
        };
        let Some(key) = self.map.snapshot.as_ref().map(|s| s.dots[i].key.clone()) else {
            return;
        };
        let Some(r) = &mut self.review else {
            return;
        };
        match r.selected.iter().position(|k| *k == key) {
            Some(at) => {
                r.selected.remove(at);
            }
            None => r.selected.push(key),
        }
        let n = r.selected.len();
        self.focus = Focus::Reader;
        self.status = format!(
            "{n} selected: m moves them, d discards them, space keeps them, y as suggested; esc lets them go"
        );
    }

    /// What a key chooses for all that is selected in the Inbox looked
    /// through, if any is: then the selection let go, and the one after
    /// the last of it picked. Whether it was its to do.
    pub(super) fn review_all(&mut self, act: Act) -> bool {
        let Some(s) = self.map.snapshot.clone() else {
            return false;
        };
        let selected: Vec<usize> = self
            .review
            .as_ref()
            .map(|r| r.selected.iter().filter_map(|k| s.find(k)).collect())
            .unwrap_or_default();
        if selected.is_empty() {
            return false;
        }
        let choice = |i: usize| match act {
            Act::Trash => Some(Choice::Discard),
            Act::ScreenDown => Some(Choice::Keep),
            Act::FileFirst => Some(Self::suggested(&s, i)),
            _ => None,
        };
        match act {
            // Chosen for several together: nothing said of each, so
            // nothing learnt.
            Act::Trash | Act::ScreenDown | Act::FileFirst => {
                for &i in &selected {
                    self.decide(i, choice(i), false);
                    self.untaught.insert(s.dots[i].key.clone());
                }
                if let Some(r) = &mut self.review {
                    r.join(selected.len());
                }
            }
            // The labels dropped down for the one picked, the rest with it.
            Act::Move => {
                let Some(i) = self.review_at().map(|(i, _)| i) else {
                    return true;
                };
                self.open_menu(i);
                if let Some(menu) = &mut self.menu {
                    menu.with = selected
                        .iter()
                        .filter(|&&j| j != i)
                        .map(|&j| s.dots[j].key.clone())
                        .collect();
                }
                return true;
            }
            Act::Back => {
                self.unselect();
                self.status = "let go".to_string();
                return true;
            }
            _ => return false,
        }
        self.after_selected(&selected);
        true
    }

    /// The selection let go, and the message after the last of `selected`
    /// picked.
    pub(super) fn after_selected(&mut self, selected: &[usize]) {
        let rows = self.review_rows();
        let last = rows.iter().rposition(|i| selected.contains(i));
        self.unselect();
        if let Some(next) = last.and_then(|at| rows.get(at + 1)) {
            self.pick_review(Some(*next));
        }
    }

    /// Let go of what is selected in the Inbox looked through.
    pub(super) fn unselect(&mut self) {
        if let Some(r) = &mut self.review {
            r.selected.clear();
        }
    }
}
