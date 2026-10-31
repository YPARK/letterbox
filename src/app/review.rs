//! The Inbox looked through beside the map: what is chosen for each
//! message, or for many selected, the labels dropped down, and done at once.
use super::*;

/// How many messages of the Inbox looked through a page goes past.
pub(super) const REVIEW_PAGE: isize = 10;

/// What is asked over the screen, one thing at a time: the keys' help,
/// scrolled so far; the form to add an account; the message's files and
/// links; which folder is Trash; whether to remove an account; what to
/// call one, as typed so far.
/// The Inbox looked through beside the map, each message shown what is
/// suggested for it: what was chosen for those looked at done at once,
/// when asked, and the rest left as they are.
#[derive(Default)]
pub struct Review {
    /// The message picked in it.
    pub picked: Option<crate::index::Key>,
    /// What was chosen for each message looked at; those not in it are
    /// left as they are.
    pub chosen: std::collections::BTreeMap<crate::index::Key, Choice>,
    /// The first message shown.
    pub top: usize,
    /// Whether the message picked is read in its place.
    pub reading: bool,
    /// The message whose label is being picked on the map.
    pub relabel: Option<crate::index::Key>,
    /// What is selected with the mouse, to choose for all of it at once.
    pub selected: Vec<crate::index::Key>,
    /// What each choice made, alone or for several at once, took the
    /// place of, the last last: C-/ takes it back before it is done.
    pub before: Vec<Vec<Was>>,
}

/// What a choice took the place of for one message: what was chosen for
/// it, if anything, and whether it was not to be learnt from.
pub type Was = (crate::index::Key, Option<Choice>, bool);

impl Review {
    /// Choose for several messages at once, each its choice or none, and
    /// each to be learnt from, `taught`, or not: what each was, choice and
    /// all, kept together, to be taken back as one.
    pub(super) fn set_all(
        &mut self,
        choices: Vec<(crate::index::Key, Option<Choice>)>,
        untaught: &mut std::collections::HashSet<crate::index::Key>,
        taught: bool,
    ) {
        let step: Vec<Was> = choices
            .into_iter()
            .map(|(key, c)| {
                let was = self.put(key.clone(), c);
                let unt = if taught {
                    untaught.remove(&key)
                } else {
                    !untaught.insert(key.clone())
                };
                (key, was, unt)
            })
            .collect();
        if !step.is_empty() {
            self.before.push(step);
        }
    }

    /// Take back the last choice made, each message's choice and whether
    /// it was to be learnt from as they were: the messages it was made
    /// for, if there was one.
    pub(super) fn take_back(
        &mut self,
        untaught: &mut std::collections::HashSet<crate::index::Key>,
    ) -> Option<Vec<crate::index::Key>> {
        let last = self.before.pop()?;
        let keys = last.iter().map(|(k, _, _)| k.clone()).collect();
        for (key, was, unt) in last.into_iter().rev() {
            if unt {
                untaught.insert(key.clone());
            } else {
                untaught.remove(&key);
            }
            self.put(key, was);
        }
        Some(keys)
    }

    /// `c` chosen for `key`, or, none, nothing; what it was.
    fn put(&mut self, key: crate::index::Key, c: Option<Choice>) -> Option<Choice> {
        match c {
            Some(c) => self.chosen.insert(key, c),
            None => self.chosen.remove(&key),
        }
    }
}

/// The labels a message may be moved to, dropped down under it, in the
/// list or the Inbox looked through: narrowed by what is typed, the one
/// picked lit.
#[derive(Clone, Debug, Default)]
pub struct Menu {
    pub key: crate::index::Key,
    /// Those to be moved with it, as many as are marked.
    pub with: Vec<crate::index::Key>,
    pub typed: String,
    pub at: usize,
}

/// How many labels are dropped down: the nearest few, or more of those
/// holding what is typed.
pub(super) const MENU_NEAREST: usize = 5;

pub(super) const MENU_TYPED: usize = 8;

/// What may be picked in the labels dropped down.
#[derive(Clone, Debug, PartialEq)]
pub enum Pick {
    /// A label, and how far it is from the message by angle.
    Label(String, f32),
    /// A label not yet made, as typed.
    New(String),
    /// Another account's folder, as `(account, path, name)`: the message
    /// moved there.
    Elsewhere(usize, String, String),
    /// Where it goes, picked on the map.
    OnMap,
}

/// What to do with a message in the Inbox.
#[derive(Clone, Debug, PartialEq)]
pub enum Choice {
    /// Filed under the label, out of the Inbox.
    Move(String),
    /// Moved to another account's folder, as `(account, path)`.
    Bring(usize, String),
    Keep,
    /// To the Trash.
    Discard,
}

/// Where a message of the Inbox looked through is heading, as chosen,
/// else as suggested.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Bound {
    /// To the cluster of its label, on the map.
    To(u32),
    Trash,
    Kept,
    /// To a label not yet on the map.
    Elsewhere,
}

/// How many are heading somewhere: chosen to, and only suggested.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct Going {
    pub chosen: usize,
    pub suggested: usize,
}

impl Going {
    pub fn all(&self) -> usize {
        self.chosen + self.suggested
    }
}

/// Where the Inbox looked through is heading: to each label's cluster, to
/// the Trash, or kept; where the one picked is.
#[derive(Clone, Debug, Default)]
pub struct Heading {
    pub to: std::collections::BTreeMap<u32, Going>,
    pub trash: Going,
    pub kept: Going,
    pub elsewhere: Going,
    pub picked: Option<Bound>,
}

impl Heading {
    /// How much of what goes to a cluster goes to `id`, as of the most to
    /// any: from 0, none, to 1.
    pub fn heat(&self, id: u32) -> f32 {
        let most = self.to.values().map(Going::all).max().unwrap_or(0);
        let here = self.to.get(&id).map_or(0, Going::all);
        if most == 0 {
            0.0
        } else {
            here as f32 / most as f32
        }
    }
}

/// What was chosen in the Inbox looked through, as asked whether to do it.
#[derive(Clone, Debug, Default)]
pub struct Plan {
    /// What is to be done, said.
    pub said: String,
    pub(super) moves: std::collections::BTreeMap<String, Vec<crate::index::Key>>,
    pub(super) discard: Vec<crate::index::Key>,
    pub(super) kept: Vec<crate::index::Key>,
    /// Each to go to another account's folder, as `(account, path)`.
    pub(super) brings: Vec<(crate::index::Key, usize, String)>,
}

impl App {
    /// Whether the Inbox looked through is beside the map, its list shown,
    /// not one of it read.
    pub fn reviewing(&self) -> bool {
        self.map.mode == Mode::Map && self.review.as_ref().is_some_and(|r| !r.reading)
    }

    /// Narrow the map to the cluster of the anchor clicked; again, to all.
    /// The messages in an Inbox, to look through: newest first, by their
    /// place in the map, those kept before after the rest; none sent from
    /// here.
    pub fn review_rows(&self) -> Vec<usize> {
        let Some(s) = &self.map.snapshot else {
            return Vec::new();
        };
        let mut rows: Vec<usize> = (0..s.dots.len())
            .filter(|&i| s.dots[i].system.inbox && !s.dots[i].system.sent)
            .collect();
        rows.sort_by_key(|&i| {
            let d = &s.dots[i];
            (
                d.looked,
                std::cmp::Reverse((d.date.unwrap_or(d.arrived), i)),
            )
        });
        rows
    }

    /// Where the Inbox looked through is heading, as it is chosen and, where
    /// not, suggested: none, not looked through.
    pub fn heading(&self) -> Option<Heading> {
        if self.map.mode != Mode::Map {
            return None;
        }
        self.review.as_ref()?;
        let s = self.map.snapshot.as_ref()?;
        let picked = self.review_at().map(|(i, _)| i);
        let mut h = Heading::default();
        for i in self.review_rows() {
            let chosen = self.chosen(i);
            let bound = match chosen.clone().unwrap_or_else(|| Self::suggested(s, i)) {
                Choice::Move(label) => s
                    .groups
                    .iter()
                    .find(|g| g.label.as_deref() == Some(label.as_str()))
                    .map_or(Bound::Elsewhere, |g| Bound::To(g.id)),
                Choice::Bring(..) => Bound::Elsewhere,
                Choice::Discard => Bound::Trash,
                Choice::Keep => Bound::Kept,
            };
            let going = match bound {
                Bound::To(id) => h.to.entry(id).or_default(),
                Bound::Trash => &mut h.trash,
                Bound::Kept => &mut h.kept,
                Bound::Elsewhere => &mut h.elsewhere,
            };
            if chosen.is_some() {
                going.chosen += 1;
            } else {
                going.suggested += 1;
            }
            if picked == Some(i) {
                h.picked = Some(bound);
            }
        }
        Some(h)
    }

    /// What is suggested for the message `i` of `s`: discarded, if mail
    /// like it is thrown away, as learnt; else filed where it would go;
    /// else kept.
    pub fn suggested(s: &map::Snapshot, i: usize) -> Choice {
        if s.dots[i].thrown.is_some() {
            return Choice::Discard;
        }
        s.suggestion(i)
            .map_or(Choice::Keep, |l| Choice::Move(l.to_string()))
    }

    /// What was chosen for the message `i` of the map, if it was looked at.
    pub fn chosen(&self, i: usize) -> Option<Choice> {
        let s = self.map.snapshot.as_ref()?;
        self.review.as_ref()?.chosen.get(&s.dots[i].key).cloned()
    }

    /// What is shown for the message `i` of the map: what was chosen, else
    /// what is suggested.
    pub fn choice(&self, i: usize) -> Choice {
        let Some(s) = &self.map.snapshot else {
            return Choice::Keep;
        };
        self.chosen(i).unwrap_or_else(|| Self::suggested(s, i))
    }

    /// Open the Inbox to look through beside the map; or close it, what was
    /// chosen kept for when it is opened again.
    pub(super) fn open_review(&mut self) {
        if let Some(r) = self.review.take() {
            self.review_kept = r.chosen;
            self.focus = Focus::List;
            self.map.moving = None;
            self.status = "the Inbox looked through, closed: i opens it as it was".to_string();
            return;
        }
        if self.map.mode != Mode::Map {
            self.show_map();
            if self.map.mode != Mode::Map {
                return;
            }
        }
        let first = self.review_rows().first().copied();
        self.review = Some(Review {
            chosen: std::mem::take(&mut self.review_kept),
            ..Review::default()
        });
        self.focus = Focus::Reader;
        self.pick_review(first);
        self.status =
            "the Inbox, to look through as far as you like: m move, space keep, d discard, y as suggested; C-c C-c does those chosen"
                .to_string();
    }

    /// Pick the message `i` of the map in the Inbox looked through, and on
    /// the map.
    pub(super) fn pick_review(&mut self, i: Option<usize>) {
        let key = i.and_then(|i| Some(self.map.snapshot.as_ref()?.dots[i].key.clone()));
        if let Some(r) = &mut self.review {
            r.picked = key.clone();
            r.reading = false;
        }
        if key.is_some() {
            self.map.cursor = key;
        }
    }

    /// The message picked in the Inbox looked through, by its place in the
    /// map, and in it.
    pub fn review_at(&self) -> Option<(usize, usize)> {
        let s = self.map.snapshot.as_ref()?;
        let i = s.find(self.review.as_ref()?.picked.as_ref()?)?;
        let at = self.review_rows().iter().position(|&j| j == i)?;
        Some((i, at))
    }

    /// Pick the message `by` after the one picked, or before.
    pub(super) fn review_step(&mut self, by: isize) {
        let rows = self.review_rows();
        if rows.is_empty() {
            return;
        }
        let at = self.review_at().map_or(0, |(_, at)| at as isize + by);
        self.pick_review(Some(rows[at.clamp(0, rows.len() as isize - 1) as usize]));
    }

    /// Choose `c` for the message `i`, or, none, leave it as it is; then,
    /// `on`, go on to the next.
    pub(super) fn decide(&mut self, i: usize, c: Option<Choice>, on: bool) {
        self.decide_all(&[i], |_| c.clone(), true);
        if on {
            self.review_step(1);
        }
    }

    /// Choose for each of the messages `all` what `c` says, or leave it as
    /// it is, each to be learnt from, `taught`, or not: taken back as one.
    pub(super) fn decide_all(
        &mut self,
        all: &[usize],
        c: impl Fn(usize) -> Option<Choice>,
        taught: bool,
    ) {
        let Some(s) = self.map.snapshot.clone() else {
            return;
        };
        let keys: Vec<_> = all.iter().map(|&i| (s.dots[i].key.clone(), c(i))).collect();
        if let Some(r) = &mut self.review {
            r.set_all(keys, &mut self.untaught, taught);
        }
    }

    pub(super) fn choose(&mut self, i: usize, c: Choice, on: bool) {
        self.decide(i, Some(c), on);
    }

    /// Drop down the labels the message `i` may be moved to.
    pub(super) fn open_menu(&mut self, i: usize) {
        let Some(key) = self.map.snapshot.as_ref().map(|s| s.dots[i].key.clone()) else {
            return;
        };
        self.menu = Some(Menu {
            key,
            ..Menu::default()
        });
        self.status = "move it to: type to narrow, ↑ ↓ pick, ⏎ moves it there, esc".to_string();
    }

    /// The labels the message dropped down for may be moved to: where it
    /// would go first, then the rest by how near they are to it by angle,
    /// the nearest few, or, typed, every one holding what is typed; a new
    /// one, if none is called that; and picking on the map.
    pub fn menu_items(&self) -> Vec<Pick> {
        let (Some(s), Some(menu)) = (self.map.snapshot.as_ref(), self.menu.as_ref()) else {
            return Vec::new();
        };
        let Some(i) = s.find(&menu.key) else {
            return Vec::new();
        };
        let typed = menu.typed.trim().to_lowercase();
        let first = s.suggestion(i).map(crate::index::fold);
        let mut labels: Vec<(String, f32)> = s
            .model
            .label_vectors
            .iter()
            .filter(|(folded, (name, _))| {
                typed.is_empty() || folded.contains(&typed) || name.to_lowercase().contains(&typed)
            })
            .map(|(_, (name, v))| (name.clone(), crate::model::degrees(&s.model.vectors[i], v)))
            .collect();
        labels.sort_by(|a, b| {
            let lead = |n: &str| Some(crate::index::fold(n)) != first;
            lead(&a.0)
                .cmp(&lead(&b.0))
                .then(a.1.total_cmp(&b.1))
                .then(a.0.cmp(&b.0))
        });
        let named = labels
            .iter()
            .any(|(n, _)| crate::index::fold(n) == crate::index::fold(&menu.typed));
        labels.truncate(if typed.is_empty() {
            MENU_NEAREST
        } else {
            MENU_TYPED
        });
        // Other accounts' folders: those named as a label shown, or holding
        // what is typed.
        let shown: Vec<String> = labels.iter().map(|(n, _)| crate::index::fold(n)).collect();
        let elsewhere: Vec<Pick> = self
            .place_of(&s.dots[i])
            .map(|(a, _, _)| self.elsewhere(a))
            .unwrap_or_default()
            .into_iter()
            .filter(|(_, path, name)| {
                if typed.is_empty() {
                    shown.contains(&crate::index::fold(name))
                } else {
                    name.to_lowercase().contains(&typed) || path.to_lowercase().contains(&typed)
                }
            })
            .take(MENU_TYPED)
            .map(|(b, path, name)| Pick::Elsewhere(b, path, name))
            .collect();
        let mut items: Vec<Pick> = labels.into_iter().map(|(n, d)| Pick::Label(n, d)).collect();
        items.extend(elsewhere);
        if !typed.is_empty() && !named {
            items.push(Pick::New(menu.typed.trim().to_string()));
        }
        items.push(Pick::OnMap);
        items
    }

    /// A key while the labels are dropped down: what is typed narrows them.
    pub(super) fn menu_key(&mut self, key: KeyEvent) {
        let count = self.menu_items().len();
        let Some(menu) = self.menu.as_mut() else {
            return;
        };
        let ctrl = key.modifiers.contains(KeyModifiers::CONTROL);
        match key.code {
            KeyCode::Esc => {
                self.menu = None;
                self.status = "not moved".to_string();
            }
            KeyCode::Char('g') if ctrl => {
                self.menu = None;
                self.status = "not moved".to_string();
            }
            // Round, past either end.
            KeyCode::Down => menu.at = (menu.at + 1) % count.max(1),
            KeyCode::Char('n') if ctrl => menu.at = (menu.at + 1) % count.max(1),
            KeyCode::Up => menu.at = (menu.at + count.max(1) - 1) % count.max(1),
            KeyCode::Char('p') if ctrl => menu.at = (menu.at + count.max(1) - 1) % count.max(1),
            KeyCode::Enter => {
                let at = menu.at;
                self.pick_item(at);
            }
            _ => {
                if edit_line(&mut menu.typed, key) {
                    menu.at = 0;
                }
            }
        }
    }

    /// Pick the item `at` of the labels dropped down: in the list, filed
    /// there at once; in the Inbox looked through, to be, with the rest; or,
    /// picking on the map, where it goes asked there.
    pub(super) fn pick_item(&mut self, at: usize) {
        let item = self.menu_items().get(at).cloned();
        let Some(menu) = self.menu.take() else {
            return;
        };
        let Some(i) = self.map.snapshot.as_ref().and_then(|s| s.find(&menu.key)) else {
            return;
        };
        if self.map.mode == Mode::List {
            match item {
                Some(Pick::Label(name, _) | Pick::New(name)) => {
                    let mut keys = vec![menu.key];
                    keys.extend(menu.with);
                    self.file_keys(keys, &name)
                }
                // To another account: each marked moved there, asked first
                // if it is more than one.
                Some(Pick::Elsewhere(to, path, _)) => {
                    let Some(s) = self.map.snapshot.clone() else {
                        return;
                    };
                    let keys: Vec<crate::index::Key> =
                        std::iter::once(menu.key).chain(menu.with).collect();
                    if keys.len() > 1 {
                        self.open_bulk(BulkAct::Elsewhere(to, path), keys);
                        return;
                    }
                    for key in &keys {
                        if let Some(j) = s.find(key) {
                            self.bring(&s.dots[j].clone(), to, &path, None);
                        }
                    }
                }
                Some(Pick::OnMap) => {
                    self.show_map();
                    self.map.cursor = Some(menu.key.clone());
                    let mut keys = vec![menu.key];
                    keys.extend(menu.with);
                    self.map.region = None;
                    self.map.moving = Some(keys);
                    self.status =
                        "where it goes: a click on a cluster, or ⏎ on one; N a new label; esc stops"
                            .to_string();
                }
                None => {}
            }
            return;
        }
        // What is chosen, and how it is said: a label, or another account's
        // folder.
        let chosen = match &item {
            Some(Pick::Label(name, _) | Pick::New(name)) => {
                Some((Choice::Move(name.clone()), name.clone()))
            }
            Some(Pick::Elsewhere(to, path, name)) => Some((
                Choice::Bring(*to, path.clone()),
                format!("{name} · {}", self.accounts[*to].name),
            )),
            _ => None,
        };
        match (item, chosen) {
            (_, Some((choice, said))) if !menu.with.is_empty() => {
                let Some(s) = self.map.snapshot.clone() else {
                    return;
                };
                let all: Vec<usize> = std::iter::once(i)
                    .chain(menu.with.iter().filter_map(|k| s.find(k)))
                    .collect();
                self.decide_all(&all, |_| Some(choice.clone()), true);
                self.status = format!(
                    "{} to go to {said} · C-c C-c does it, with the rest",
                    all.len()
                );
                self.after_selected(&all);
            }
            (_, Some((choice, said))) => {
                self.status = format!("to go to {said} · C-c C-c does it, with the rest");
                self.choose(i, choice, true);
            }
            (Some(Pick::OnMap), _) => self.pick_on_map(i),
            _ => {}
        }
    }

    /// Pick where the message `i` goes on the map: a click on a cluster,
    /// or ⏎ on one.
    pub(super) fn pick_on_map(&mut self, i: usize) {
        let Some(key) = self.map.snapshot.as_ref().map(|s| s.dots[i].key.clone()) else {
            return;
        };
        if let Some(r) = &mut self.review {
            r.relabel = Some(key.clone());
        }
        self.map.cursor = Some(key.clone());
        self.map.moving = Some(vec![key]);
        self.map.close_tree();
        self.focus = Focus::List;
        self.status = "where it goes: a click on a cluster, or ⏎ on one; N a new label; esc stops"
            .to_string();
    }

    /// Move the message `i` under a new label, its name asked: chosen, to be
    /// made as it is filed.
    pub(super) fn choose_new(&mut self, i: usize) {
        let Some(key) = self.map.snapshot.as_ref().map(|s| s.dots[i].key.clone()) else {
            return;
        };
        if let Some(r) = &mut self.review {
            r.relabel = Some(key.clone());
        }
        self.map.moving = Some(vec![key]);
        self.ask_new_label();
    }

    /// Do what a key does in the Inbox looked through, with the focus on
    /// it; whether it was its to do.
    pub(super) fn review_act(&mut self, act: Act) -> bool {
        if self.focus != Focus::Reader || self.map.mode != Mode::Map {
            return false;
        }
        let Some(r) = &mut self.review else {
            return false;
        };
        // Reading one, esc goes back to the rest; what may be done with it
        // is chosen as it is beside the rest, and the rest shown again.
        // What was chosen and not yet done, taken back first; with none,
        // what was done, as anywhere.
        if act == Act::Undo {
            return self.unchoose();
        }
        if r.reading {
            match act {
                Act::Back => r.reading = false,
                Act::Review => self.open_review(),
                Act::Trash | Act::Move | Act::FileFirst | Act::FileAll | Act::NewLabel => {
                    r.reading = false;
                    return self.review_act(act);
                }
                // Nothing is done till C-c C-c, read or not.
                Act::Archive => {
                    self.status =
                        "here, m moves it, space keeps it, d discards it; C-c C-c does those chosen"
                            .to_string()
                }
                _ => return false,
            }
            return true;
        }
        let picked = self.review_at().map(|(i, _)| i);
        if self.review_all(act) {
            return true;
        }
        match (act, picked) {
            (Act::Down, _) => self.review_step(1),
            (Act::Up, _) => self.review_step(-1),
            (Act::Top, _) => self.review_step(isize::MIN / 2),
            (Act::End, _) => self.review_step(isize::MAX / 2),
            (Act::PageDown, _) => self.review_step(REVIEW_PAGE),
            (Act::PageUp, _) => self.review_step(-REVIEW_PAGE),
            (Act::Left | Act::Right, Some(i)) => {
                let label = match self.choice(i) {
                    Choice::Move(l) => Some(l),
                    _ => self
                        .map
                        .snapshot
                        .as_ref()
                        .and_then(|s| s.suggestion(i))
                        .map(str::to_string),
                };
                // Round them, and round to none chosen.
                let mut all: Vec<Option<Choice>> = vec![None];
                all.extend(label.into_iter().map(|l| Some(Choice::Move(l))));
                all.extend([Some(Choice::Keep), Some(Choice::Discard)]);
                let now = self.chosen(i);
                let at = all.iter().position(|c| *c == now).unwrap_or(0) as isize;
                let by = if act == Act::Right { 1 } else { -1 };
                let next = all[(at + by).rem_euclid(all.len() as isize) as usize].clone();
                self.decide(i, next, false);
            }
            (Act::Move, Some(i)) => self.open_menu(i),
            (Act::NewLabel, Some(i)) => self.choose_new(i),
            (Act::ScreenDown, Some(i)) => self.choose(i, Choice::Keep, true),
            (Act::Trash, Some(i)) => self.choose(i, Choice::Discard, true),
            (Act::FileAll, Some(i)) => self.choose_like(i),
            (Act::FileFirst, Some(i)) => {
                let suggested = self
                    .map
                    .snapshot
                    .as_ref()
                    .map_or(Choice::Keep, |s| Self::suggested(s, i));
                // The filter's own guess, taken as it was: not learnt from,
                // or it would teach itself.
                let guessed = suggested == Choice::Discard;
                self.decide_all(&[i], |_| Some(suggested.clone()), !guessed);
                self.review_step(1);
            }
            (Act::Open, Some(_)) => {
                self.read_dot();
                if let Some(r) = &mut self.review {
                    r.reading = true;
                }
            }
            (Act::Review, _) => self.open_review(),
            // esc goes back to the map, all of it kept.
            (Act::Back, _) => self.focus = Focus::List,
            (Act::Seen, Some(_)) => self.dot_flag(Flag::Seen),
            (Act::Flag, Some(_)) => self.dot_flag(Flag::Flagged),
            (Act::Archive, _) => {
                self.status =
                    "here, m moves it, space keeps it, d discards it; C-c C-c does those chosen"
                        .to_string()
            }
            (
                Act::Left
                | Act::Right
                | Act::Move
                | Act::NewLabel
                | Act::ScreenDown
                | Act::Trash
                | Act::FileFirst
                | Act::FileAll
                | Act::Open,
                None,
            ) => {}
            _ => return false,
        }
        true
    }

    /// How many of the Inbox looked through are chosen to be filed,
    /// discarded and kept, and how many are left as they are.
    pub fn review_counts(&self) -> (usize, usize, usize, usize) {
        let (mut filed, mut discarded, mut kept, mut left) = (0, 0, 0, 0);
        for i in self.review_rows() {
            match self.chosen(i) {
                Some(Choice::Move(_) | Choice::Bring(..)) => filed += 1,
                Some(Choice::Discard) => discarded += 1,
                Some(Choice::Keep) => kept += 1,
                None => left += 1,
            }
        }
        (filed, discarded, kept, left)
    }

    /// Ask whether to do what was chosen in the Inbox looked through.
    pub(super) fn ask_apply(&mut self) {
        // Done once what was done last is confirmed, or failed.
        if self.batch.busy() {
            self.status = format!(
                "still applying what was chosen before: {}",
                self.batch.said()
            );
            return;
        }
        let Some(s) = self.map.snapshot.clone() else {
            return;
        };
        let mut plan = Plan::default();
        let mut left = 0;
        for i in self.review_rows() {
            let key = s.dots[i].key.clone();
            match self.chosen(i) {
                Some(Choice::Move(label)) => plan.moves.entry(label).or_default().push(key),
                Some(Choice::Bring(to, path)) => plan.brings.push((key, to, path)),
                Some(Choice::Discard) => plan.discard.push(key),
                Some(Choice::Keep) => plan.kept.push(key),
                None => left += 1,
            }
        }
        if plan.moves.is_empty()
            && plan.discard.is_empty()
            && plan.kept.is_empty()
            && plan.brings.is_empty()
        {
            self.status =
                "nothing chosen yet: y as suggested, m, space or d; C-c C-c does only those"
                    .to_string();
            return;
        }
        let mut said: Vec<String> = plan
            .moves
            .iter()
            .map(|(label, keys)| format!("file {} under {label}", keys.len()))
            .collect();
        if !plan.brings.is_empty() {
            said.push(format!(
                "move {} to other accounts, their originals to the Trash once copied",
                plan.brings.len()
            ));
        }
        if !plan.discard.is_empty() {
            said.push(format!("discard {} to the Trash", plan.discard.len()));
        }
        if !plan.kept.is_empty() {
            said.push(format!("keep {} in the Inbox", plan.kept.len()));
        }
        let mut said = said.join(", ");
        if let Some(first) = said.get(..1) {
            said.replace_range(..1, &first.to_uppercase());
        }
        said.push('.');
        match left {
            0 => {}
            1 => said.push_str(" The other one is left as it is."),
            _ => said.push_str(&format!(" The other {left} are left as they are.")),
        }
        plan.said = said;
        self.dialog = Dialog::Apply(Box::new(plan));
    }

    /// Do what was chosen in the Inbox looked through, as `plan` said it:
    /// each to be moved filed under its label, each to be discarded moved
    /// to the Trash, each kept learnt as kept (and after the rest when
    /// looked through again); those not looked at left as they are, and
    /// what was chosen since still chosen.
    pub(super) fn apply_review(&mut self, plan: Plan) {
        let Plan {
            moves,
            discard,
            kept,
            brings,
            ..
        } = plan;
        // What was decided alone is learnt; the rest is not.
        let untaught = std::mem::take(&mut self.untaught);
        let alone = |k: &crate::index::Key| !untaught.contains(k);
        let kept_alone: Vec<crate::index::Key> =
            kept.iter().filter(|k| alone(k)).cloned().collect();
        self.teach(&kept_alone, false);
        if let Some(r) = &mut self.review {
            r.relabel = None;
            r.before.clear();
            let brought = brings.iter().map(|(k, _, _)| k);
            for key in moves
                .values()
                .flatten()
                .chain(&discard)
                .chain(&kept)
                .chain(brought)
            {
                r.chosen.remove(key);
            }
        }
        // Each to another account: copied there, then its original trashed.
        if let Some(s) = self.map.snapshot.clone() {
            for (key, to, path) in &brings {
                if let Some(i) = s.find(key) {
                    let choice = Choice::Bring(*to, path.clone());
                    self.bring(&s.dots[i].clone(), *to, path, Some(choice));
                }
            }
        }
        self.review_kept.clear();
        // What was chosen for each, kept with what is asked, to be chosen
        // again if it fails.
        self.choosing = moves
            .iter()
            .flat_map(|(label, keys)| {
                keys.iter()
                    .map(|k| (k.clone(), Choice::Move(label.clone())))
            })
            .chain(discard.iter().map(|k| (k.clone(), Choice::Discard)))
            .collect();
        let filed: usize = moves.values().map(Vec::len).sum();
        let kept_n = kept.len();
        // Asked already, by C-c C-c: those decided alone learnt.
        for (label, keys) in moves {
            let (taught, not): (Vec<_>, Vec<_>) = keys.into_iter().partition(|k| alone(k));
            self.file_now(taught, &label, true);
            self.file_now(not, &label, false);
        }
        self.map.suggesting = None;
        let mut said = format!("{filed} filed");
        if kept_n > 0 {
            said = format!("{said} · {kept_n} kept");
        }
        if !discard.is_empty() {
            self.map.region = Some(map::Region { keys: discard });
            self.region_trash(&alone);
            said = format!("{said} · {}", self.status);
        }
        self.status = said;
        self.choosing.clear();
        let first = self.review_rows().first().copied();
        self.pick_review(first);
    }

    /// A click on a message in the Inbox looked through picks it; on the
    /// one picked, reads it.
    pub(super) fn review_click(&mut self, row: usize) {
        let Some(&i) = self.review_rows().get(row) else {
            return;
        };
        self.focus = Focus::Reader;
        if self.review_at().is_some_and(|(j, _)| j == i) {
            self.review_act(Act::Open);
        } else {
            self.pick_review(Some(i));
        }
    }

    /// A click on one of what may be done with a message looked through.
    pub(super) fn choice_click(&mut self, row: usize, which: u8) {
        let Some(&i) = self.review_rows().get(row) else {
            return;
        };
        self.focus = Focus::Reader;
        self.pick_review(Some(i));
        // A click on what was chosen leaves it as it is again.
        let toggle = |app: &mut Self, c: Choice| {
            let c = (app.chosen(i) != Some(c.clone())).then_some(c);
            app.decide(i, c, false);
        };
        match which {
            0 => self.open_menu(i),
            1 => toggle(self, Choice::Keep),
            2 => toggle(self, Choice::Discard),
            _ => self.choose_new(i),
        }
    }
}
