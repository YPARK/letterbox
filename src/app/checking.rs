//! The grammar of the message being written, checked: sent to the model's
//! thread, read the first time it is asked for, and each change it would
//! make gone through, as Emacs's query-replace goes through them.
use super::*;
use crate::grammar::{self, Edit};

/// The changes a model would make, and which is being looked at.
pub struct Checking {
    pub edits: Vec<Edit>,
    pub at: usize,
    /// How many were taken.
    pub taken: usize,
}

impl Checking {
    /// The change looked at.
    pub fn edit(&self) -> Option<&Edit> {
        self.edits.get(self.at)
    }
}

impl App {
    /// Check what is written: sent to the model's thread, made the first
    /// time with the model read on it, so the screen never waits for it.
    pub(super) fn check_grammar(&mut self) {
        let Some(compose) = &self.compose else {
            return;
        };
        let text = compose.buffer.text();
        self.checking = None;
        let first = self.checker.is_none();
        if first {
            let (to, asked) = std::sync::mpsc::channel::<String>();
            let (open, name, replies) = (
                self.grammar_open,
                self.grammar_model.clone(),
                self.replies_to.clone(),
            );
            std::thread::spawn(move || {
                let mut model = open(&name).map_err(|e| format!("{e:#}"));
                for text in asked {
                    let lines: Vec<String> = text.lines().map(str::to_string).collect();
                    let edits = match &mut model {
                        Ok(m) => grammar::check(&lines, m.as_mut()).map_err(|e| format!("{e:#}")),
                        Err(why) => Err(why.clone()),
                    };
                    let answer = Answer::Checked { text, edits };
                    if replies
                        .send(Reply {
                            account: usize::MAX,
                            answer,
                        })
                        .is_err()
                    {
                        return;
                    }
                }
            });
            self.checker = Some(to);
        }
        if self.checker.as_ref().is_some_and(|c| c.send(text).is_ok()) {
            self.pending += 1;
            self.status = if first {
                format!(
                    "checking the grammar · {} read the first time…",
                    self.grammar_model
                )
            } else {
                "checking the grammar…".to_string()
            };
        } else {
            self.checker = None;
            self.status = "the grammar cannot be checked now: M-$ again".to_string();
        }
    }

    /// What the model would change of `text`: gone through one by one, if
    /// the message is still as it was sent.
    pub(super) fn checked(&mut self, text: &str, edits: Result<Vec<Edit>, String>) {
        let Some(compose) = &self.compose else {
            return;
        };
        if compose.buffer.text() != text {
            self.status = "changed while it was checked: M-$ again".to_string();
            return;
        }
        match edits {
            Err(why) => self.status = first_line(&why).to_string(),
            Ok(edits) if edits.is_empty() => {
                self.status = "✓ the grammar: nothing to change".to_string()
            }
            Ok(edits) => {
                self.checking = Some(Checking {
                    edits,
                    at: 0,
                    taken: 0,
                });
                self.say_edit();
            }
        }
    }

    /// The change looked at, the cursor on it, and what may be done.
    fn say_edit(&mut self) {
        let Some(c) = &self.checking else {
            return;
        };
        let Some(e) = c.edit() else {
            let taken = c.taken;
            self.checking = None;
            self.status = format!("the grammar: {taken} changed");
            return;
        };
        let (said, with) = (e.said.trim(), e.with.trim());
        let what = match (said.is_empty(), with.is_empty()) {
            (true, _) => format!("put in “{with}”"),
            (_, true) => format!("take out “{said}”"),
            _ => format!("“{said}” → “{with}”"),
        };
        let (row, col) = (e.row, e.start);
        self.status = format!(
            "{what} · y takes it, n leaves it, ! all, q stops ({} of {})",
            c.at + 1,
            c.edits.len()
        );
        if let Some(compose) = &mut self.compose {
            compose.buffer.place(row, col);
        }
    }

    /// A key while the model's changes are gone through: `y` or space takes
    /// the one looked at, `n` or ⌫ leaves it, `!` takes all that are left,
    /// `q`, ⏎, esc or `C-g` stops. Any other key stops and is the editor's:
    /// whether it was theirs.
    pub(super) fn checking_key(&mut self, key: KeyEvent) -> bool {
        let ctrl = key.modifiers.contains(KeyModifiers::CONTROL);
        let plain = key.modifiers.is_empty() || key.modifiers == KeyModifiers::SHIFT;
        match key.code {
            KeyCode::Char('y' | ' ') if plain => self.take_edit(),

            KeyCode::Char('n') | KeyCode::Backspace | KeyCode::Delete if !ctrl => self.skip_edit(),
            KeyCode::Char('!') if plain => {
                while self.checking.as_ref().is_some_and(|c| c.edit().is_some()) {
                    self.take_edit();
                }
            }
            KeyCode::Char('q') if plain => self.stop_checking(),
            KeyCode::Enter | KeyCode::Esc => self.stop_checking(),
            KeyCode::Char('g') if ctrl => self.stop_checking(),
            _ => {
                self.checking = None;
                return false;
            }
        }
        self.say_edit();
        true
    }

    /// The change looked at made: those after it on its row moved as far.
    fn take_edit(&mut self) {
        let (Some(c), Some(compose)) = (&mut self.checking, &mut self.compose) else {
            return;
        };
        let Some(e) = c.edits.get(c.at).cloned() else {
            return;
        };
        compose.buffer.replace(e.row, e.start, e.end, &e.with);
        let grown = e.with.chars().count() as isize - (e.end - e.start) as isize;
        for later in c.edits[c.at + 1..]
            .iter_mut()
            .filter(|l| l.row == e.row && l.start >= e.end)
        {
            later.start = later.start.saturating_add_signed(grown);
            later.end = later.end.saturating_add_signed(grown);
        }
        c.at += 1;
        c.taken += 1;
    }

    fn skip_edit(&mut self) {
        if let Some(c) = &mut self.checking {
            c.at += 1;
        }
    }

    /// No more gone through: what is left, left.
    fn stop_checking(&mut self) {
        if let Some(c) = &mut self.checking {
            c.at = c.edits.len();
        }
    }
}
