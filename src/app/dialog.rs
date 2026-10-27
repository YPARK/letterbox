//! The dialogs over the screen, and what a key or a click does in each.
use super::*;

/// What a click lands on, as the screen was last drawn.
#[derive(Clone, Copy, PartialEq, Debug)]
pub enum Target {
    Search,
    Side,
    Row(usize),
    List,
    Message(usize),
    Reader,
    Popup,
    /// A field of the form, by its place.
    Field(usize),
    Button(Button),
    /// A key's hint at the foot: clicked, as if it were pressed.
    Key(KeyEvent),
    /// Two keys pressed in turn, as `C-c C-c`, clicked.
    Chord(KeyEvent, KeyEvent),
    /// The line between the middle pane and the message read: dragged, it
    /// makes the message wider or narrower.
    Divider,
    /// The map, where its messages are; and a cluster's anchor on it, by
    /// the cluster's number.
    Map,
    Anchor(u32),
    /// A branch of a cluster's tree, by its row.
    Branch(usize),
    /// A message in the Inbox looked through, by its row; and one of what
    /// may be done with it: moved, kept or discarded.
    ReviewRow(usize),
    Choose(usize, u8),
    /// One of the labels dropped down, by its place.
    MenuItem(usize),
    /// An attachment or link in the list of them, by its place.
    Item(usize),
}

#[derive(Clone, Copy, PartialEq, Debug)]
pub enum Button {
    Add,
    Cancel,
    Remove,
    Keep,
    /// Do what was chosen in the Inbox looked through.
    Apply,
    NotYet,
}

/// What the message read has to open: an attachment, by its place among
/// its attachments, or a link.
#[derive(Clone, Debug, PartialEq)]
pub enum Item {
    /// The message itself, as its HTML, in the browser.
    Page,
    /// The message itself, presented by ohp.
    Present,
    File {
        at: usize,
        name: String,
        mime: String,
        size: usize,
    },
    Link(String),
}

#[derive(Default)]
pub enum Dialog {
    #[default]
    Closed,
    Help(usize),
    Form(Box<Form>),
    Files(Files),
    Trash(TrashPick),
    Remove(usize),
    Rename(usize, String),
    /// The name of a new label for what is moving, as typed so far.
    Label(String),
    /// What to call a label, as typed so far.
    RenameLabel(String, String),
    /// Files to attach to the message being written, as they are picked.
    Attach(Box<crate::pick::Picker>),
    /// The folder to save the message's files in, as it is picked.
    SaveTo(Box<SaveTo>),
    /// What the map shows, and how, to be changed: `keymap::VIEW`.
    View,
    /// Whether to begin the map, asked the first time it is wanted.
    StartMap,
    /// What was thrown away on its own, and why, to put back.
    Debrief(Box<crate::filter::Debrief>),
    /// The papers of the message read, each where it stands with
    /// paperstack, to put there.
    Papers(Box<PapersView>),
    /// Whether to do what was chosen in the Inbox looked through: what it
    /// is, as said, and only that is done.
    Apply(Box<Plan>),
    /// How what was asked of the servers went, once it all settled.
    Applied(Box<Applied>),
}

/// How what was asked of the servers went: how many were confirmed, and
/// each that failed, with why.
#[derive(Clone, Debug, Default)]
pub struct Applied {
    pub said: String,
    pub failed: Vec<String>,
}

impl Dialog {
    pub fn is_open(&self) -> bool {
        !matches!(self, Dialog::Closed)
    }

    pub fn form(&self) -> Option<&Form> {
        match self {
            Dialog::Form(form) => Some(form),
            _ => None,
        }
    }

    pub(super) fn form_mut(&mut self) -> Option<&mut Form> {
        match self {
            Dialog::Form(form) => Some(form),
            _ => None,
        }
    }

    #[cfg(test)]
    pub fn files(&self) -> Option<&Files> {
        match self {
            Dialog::Files(files) => Some(files),
            _ => None,
        }
    }

    /// The files listed: shown, or put by while the folder to save in is
    /// picked.
    pub(super) fn files_mut(&mut self) -> Option<&mut Files> {
        match self {
            Dialog::Files(files) => Some(files),
            Dialog::SaveTo(s) => Some(&mut s.files),
            _ => None,
        }
    }

    /// The folder being picked to save in, if one is.
    #[cfg(test)]
    pub fn saving(&self) -> Option<&crate::pick::Picker> {
        match self {
            Dialog::SaveTo(s) => Some(&s.picker),
            _ => None,
        }
    }

    /// The name to save under, being typed, if it is.
    #[cfg(test)]
    pub fn naming(&self) -> Option<&Name> {
        match self {
            Dialog::SaveTo(s) => s.naming.as_ref().map(|(_, name)| name),
            _ => None,
        }
    }

    #[cfg(test)]
    pub fn trash_mut(&mut self) -> Option<&mut TrashPick> {
        match self {
            Dialog::Trash(pick) => Some(pick),
            _ => None,
        }
    }

    /// Move in it: scroll the help, or pick another item, or field.
    pub(super) fn step(&mut self, by: isize) {
        let (at, len) = match self {
            Dialog::Help(scroll) => {
                *scroll = scroll.saturating_add_signed(by);
                return;
            }
            Dialog::Form(form) if !form.checking => return form.step(by),
            Dialog::Files(files) => (&mut files.at, files.items.len()),
            Dialog::Trash(pick) => (&mut pick.at, pick.folders.len()),
            Dialog::Attach(picker) => return picker.step(by),
            Dialog::SaveTo(s) => return s.picker.step(by),
            _ => return,
        };
        *at = at.saturating_add_signed(by).min(len.saturating_sub(1));
    }
}

impl App {
    /// Take a key in the dialog open.
    pub(super) fn dialog_key(&mut self, key: KeyEvent) {
        // C-g, as Emacs has it, is esc to every dialog.
        let key = if key.modifiers.contains(KeyModifiers::CONTROL) && key.code == KeyCode::Char('g')
        {
            KeyEvent::new(KeyCode::Esc, KeyModifiers::NONE)
        } else {
            key
        };
        match &mut self.dialog {
            Dialog::Closed => {}
            Dialog::Help(_) => match keymap::act(key) {
                Some(Act::Down) => self.dialog.step(1),
                Some(Act::Up) => self.dialog.step(-1),
                Some(Act::ScreenDown | Act::PageDown) => self.dialog.step(10),
                _ => self.dialog = Dialog::Closed,
            },
            Dialog::Form(_) => self.form_key(key),
            Dialog::Files(_) => self.files_key(key),
            Dialog::Attach(_) => self.attach_key(key),
            Dialog::SaveTo(_) => self.save_to_key(key),
            Dialog::View => self.view_key(key),
            Dialog::Debrief(_) => self.debrief_key(key),
            Dialog::Papers(_) => self.papers_key(key),
            Dialog::StartMap => match key.code {
                KeyCode::Char('y' | 'Y') => self.start_map(),
                _ => {
                    self.dialog = Dialog::Closed;
                    self.status = "the map is not begun: M-g asks again".to_string();
                }
            },
            Dialog::Trash(_) => match keymap::act(key) {
                Some(Act::Down) => self.dialog.step(1),
                Some(Act::Up) => self.dialog.step(-1),
                Some(Act::Open) => self.choose_trash(),
                Some(Act::Back) => self.cancel_trash(),
                _ => {}
            },
            Dialog::Rename(a, typed) => match key.code {
                KeyCode::Esc => self.dialog = Dialog::Closed,
                KeyCode::Enter => {
                    let (a, typed) = (*a, std::mem::take(typed));
                    self.dialog = Dialog::Closed;
                    self.rename(a, &typed);
                }
                _ => {
                    edit_line(typed, key);
                }
            },
            Dialog::Label(typed) => match key.code {
                KeyCode::Esc => {
                    self.dialog = Dialog::Closed;
                    // Named in the list: let go.
                    if self.map.mode == Mode::List {
                        self.map.moving = None;
                        self.status = "not moved".to_string();
                    }
                    // Named for a message looked through: back to it.
                    if let Some(r) = &mut self.review
                        && r.relabel.take().is_some()
                    {
                        self.map.moving = None;
                        self.focus = Focus::Reader;
                    }
                }
                KeyCode::Enter => {
                    let typed = std::mem::take(typed);
                    self.dialog = Dialog::Closed;
                    let keys = self.map.moving.take().unwrap_or_default();
                    self.file_keys(keys, typed.trim());
                }
                _ => {
                    edit_line(typed, key);
                }
            },
            Dialog::RenameLabel(old, typed) => match key.code {
                KeyCode::Esc => self.dialog = Dialog::Closed,
                KeyCode::Enter => {
                    let (old, typed) = (std::mem::take(old), std::mem::take(typed));
                    self.dialog = Dialog::Closed;
                    self.rename_label(&old, typed.trim());
                }
                _ => {
                    edit_line(typed, key);
                }
            },
            // Any key closes how it went; what settled since, said next.
            Dialog::Applied(_) => {
                self.dialog = Dialog::Closed;
                self.say_settled();
            }
            Dialog::Apply(_) => {
                let Dialog::Apply(plan) = std::mem::take(&mut self.dialog) else {
                    unreachable!()
                };
                if matches!(key.code, KeyCode::Char('y' | 'Y')) && key.modifiers.is_empty() {
                    self.apply_review(*plan);
                } else {
                    self.status = "nothing done · $ when it is as you want it".to_string();
                }
            }
            &mut Dialog::Remove(a) => {
                self.dialog = Dialog::Closed;
                if key.code == KeyCode::Char('y') {
                    self.remove(a);
                } else {
                    self.status = format!("kept {}", self.accounts[a].name);
                }
            }
        }
    }

    /// Take a click in the dialog open: on it, or away from it.
    pub(super) fn dialog_click(&mut self, at: Option<Target>) {
        match (&mut self.dialog, at) {
            (_, Some(Target::Popup)) => {}
            // A key at the foot, or on the dialog's border, pressed.
            (_, Some(Target::Key(key))) => self.dialog_key(key),
            (_, Some(Target::Chord(a, b))) => {
                self.dialog_key(a);
                self.dialog_key(b);
            }
            (Dialog::Closed, _) => {}
            // Away from help, or from renaming, closes it, the name left
            // as it is.
            (
                Dialog::Help(_) | Dialog::Rename(..) | Dialog::Label(_) | Dialog::RenameLabel(..),
                _,
            ) => self.dialog = Dialog::Closed,
            (Dialog::Trash(pick), Some(Target::Item(i))) => {
                pick.at = i;
                self.choose_trash();
            }
            (Dialog::Trash(_), _) => self.cancel_trash(),
            (Dialog::Files(files), Some(Target::Item(i))) => {
                files.at = i;
                if files.saving {
                    self.pick_save_dir(false);
                } else {
                    self.open_item();
                }
            }
            (Dialog::Files(_), _) => self.dialog = Dialog::Closed,
            (Dialog::Attach(picker), Some(Target::Item(i))) => {
                picker.click(i);
            }
            (Dialog::Attach(_), _) => self.dialog = Dialog::Closed,
            (Dialog::SaveTo(s), Some(Target::Item(i))) => {
                let picked = s.picker.click(i);
                self.saved_to(picked);
            }
            (Dialog::SaveTo(_), _) => self.saved_to(crate::pick::Picked::Cancel),
            (Dialog::View, Some(Target::Item(i))) => {
                if let Some(&(_, act, _)) = keymap::VIEW.get(i) {
                    self.view(act);
                }
            }
            (Dialog::View, _) => self.dialog = Dialog::Closed,
            (Dialog::Debrief(d), Some(Target::Item(i))) => {
                d.at = i;
                self.put_back_thrown(&[i]);
            }
            (Dialog::Debrief(_), _) => self.dialog = Dialog::Closed,
            (Dialog::Papers(p), Some(Target::Item(i))) => {
                p.at = i;
                self.shelve();
            }
            (Dialog::Papers(_), _) => self.dialog = Dialog::Closed,
            (Dialog::StartMap, _) => {
                self.dialog = Dialog::Closed;
                self.status = "the map is not begun: M-g asks again".to_string();
            }
            (Dialog::Form(form), Some(Target::Field(i))) if !form.checking => {
                if i == KEEP && form.at == KEEP {
                    form.turn(1);
                }
                if i == SIGN_IN && form.at == SIGN_IN {
                    form.flip();
                }
                let left = form.at == EMAIL && i != EMAIL;
                form.at = i;
                if left {
                    self.ask_microsoft();
                }
            }
            (Dialog::Form(form), Some(Target::Button(Button::Add))) if !form.checking => {
                self.submit()
            }
            (Dialog::Form(_), Some(Target::Button(Button::Cancel))) => {
                self.form_key(KeyEvent::new(KeyCode::Esc, KeyModifiers::NONE))
            }
            (Dialog::Form(_), _) => {}
            (Dialog::Remove(_), Some(Target::Button(Button::Remove))) => {
                self.dialog_key(KeyEvent::new(KeyCode::Char('y'), KeyModifiers::NONE))
            }
            (Dialog::Remove(_), Some(Target::Button(Button::Keep))) => {
                self.dialog_key(KeyEvent::new(KeyCode::Esc, KeyModifiers::NONE))
            }
            (Dialog::Remove(_), _) => {}
            (Dialog::Apply(_), Some(Target::Button(Button::Apply))) => {
                self.dialog_key(KeyEvent::new(KeyCode::Char('y'), KeyModifiers::NONE))
            }
            (Dialog::Apply(_), Some(Target::Button(Button::NotYet))) => {
                self.dialog_key(KeyEvent::new(KeyCode::Esc, KeyModifiers::NONE))
            }
            (Dialog::Apply(_), _) => {}
            (Dialog::Applied(_), _) => {
                self.dialog = Dialog::Closed;
                self.say_settled();
            }
        }
    }

    /// The account `D` and `R` act on: the one picked in the sidebar, or,
    /// with the focus elsewhere, the one listed, else the message's.
    pub(super) fn account_picked(&self) -> Option<usize> {
        match (self.focus, self.rows.get(self.row)) {
            (Focus::Side, Some(&Row::Account(a) | &Row::Folder(a, _))) => Some(a),
            (Focus::Side, _) => None,
            _ => self
                .listing
                .as_ref()
                .and_then(Listing::account)
                .or_else(|| self.current().map(|m| m.account)),
        }
    }

    /// Ask whether to remove the account picked.
    pub(super) fn ask_to_remove(&mut self) {
        let Some(a) = self.account_picked() else {
            return;
        };
        if self.editable {
            self.dialog = Dialog::Remove(a);
        } else {
            self.status = "the demo's account is made up: it cannot be removed".to_string();
        }
    }

    /// Ask what to call the account picked.
    pub(super) fn ask_to_rename(&mut self) {
        let Some(a) = self.account_picked() else {
            self.status = "pick an account's folder to rename it".to_string();
            return;
        };
        if self.editable {
            self.dialog = Dialog::Rename(a, self.accounts[a].name.clone());
        } else {
            self.status = "the demo's account is made up: it cannot be renamed".to_string();
        }
    }

    /// Call the account `a` `name`, in the accounts file and here.
    pub(super) fn rename(&mut self, a: usize, name: &str) {
        let old = self.accounts[a].name.clone();
        match config::rename(&self.config, &old, name) {
            Ok(name) => {
                self.status = format!("renamed {old} to {name}");
                self.accounts[a].name = name;
            }
            Err(e) => self.status = format!("cannot rename {old}: {e:#}"),
        }
    }
}
