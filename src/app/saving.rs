//! A message's files saved: picked from the files listed to open, or from
//! a dialog of their own; into a folder picked; and one of them under a
//! name looked at first, its stem picked to type over.
use super::*;

/// The folder to save the message's files in, being picked: all of them,
/// or the one picked in the files listed, which are shown again after.
pub struct SaveTo {
    pub picker: crate::pick::Picker,
    pub files: Files,
    pub all: bool,
    /// Whether the files were shown before, to go back to when let go:
    /// not when S, from the message, saves them all.
    pub listed: bool,
    /// Once its folder is picked, the name to save the one picked under:
    /// the folder, and the name as it is typed.
    pub naming: Option<(PathBuf, Name)>,
}

/// A name being typed: its text, the cursor, in chars, and where what is
/// picked begins, if any is, the cursor at its other end. Typed, what is
/// picked is typed over.
pub struct Name {
    pub text: String,
    pub at: usize,
    from: Option<usize>,
}

impl Name {
    /// `name`, its stem picked: all but its extension, if it has one.
    pub fn new(name: &str) -> Name {
        let stem = match name.rfind('.') {
            Some(dot) if dot > 0 => name[..dot].chars().count(),
            _ => name.chars().count(),
        };
        Name {
            text: name.to_string(),
            at: stem,
            from: Some(0),
        }
    }

    /// What is picked, from its first char to the one after its last.
    pub fn range(&self) -> Option<(usize, usize)> {
        self.from
            .map(|f| (f.min(self.at), f.max(self.at)))
            .filter(|(a, b)| a < b)
    }

    /// The text picked.
    #[cfg(test)]
    pub fn picked(&self) -> &str {
        self.range()
            .map_or("", |(a, b)| &self.text[self.byte(a)..self.byte(b)])
    }

    fn len(&self) -> usize {
        self.text.chars().count()
    }

    /// Where the char `c` begins in the text.
    fn byte(&self, c: usize) -> usize {
        self.text
            .char_indices()
            .nth(c)
            .map_or(self.text.len(), |(i, _)| i)
    }

    /// Cut what is picked, the cursor where it was; whether any was.
    fn cut(&mut self) -> bool {
        let range = self.range();
        self.from = None;
        let Some((a, b)) = range else {
            return false;
        };
        let (a_byte, b_byte) = (self.byte(a), self.byte(b));
        self.text.replace_range(a_byte..b_byte, "");
        self.at = a;
        true
    }

    /// Move the cursor to `at`, what was picked let go.
    fn go(&mut self, at: usize) {
        self.from = None;
        self.at = at.min(self.len());
    }

    pub fn key(&mut self, key: KeyEvent) {
        let ctrl = key.modifiers.contains(KeyModifiers::CONTROL);
        let alt = key.modifiers.contains(KeyModifiers::ALT);
        let range = self.range();
        match key.code {
            KeyCode::Char('u') if ctrl => {
                self.text.clear();
                self.go(0);
            }
            KeyCode::Home => self.go(0),
            KeyCode::Char('a') if ctrl => self.go(0),
            KeyCode::End => self.go(self.len()),
            KeyCode::Char('e') if ctrl => self.go(self.len()),
            // Off what is picked, to its start or its end.
            KeyCode::Left => self.go(range.map_or(self.at.saturating_sub(1), |r| r.0)),
            KeyCode::Char('b') if ctrl => self.go(range.map_or(self.at.saturating_sub(1), |r| r.0)),
            KeyCode::Right => self.go(range.map_or(self.at + 1, |r| r.1)),
            KeyCode::Char('f') if ctrl => self.go(range.map_or(self.at + 1, |r| r.1)),
            KeyCode::Backspace => {
                if !self.cut() && self.at > 0 {
                    self.at -= 1;
                    let i = self.byte(self.at);
                    self.text.remove(i);
                }
            }
            KeyCode::Delete => {
                if !self.cut() && self.at < self.len() {
                    let i = self.byte(self.at);
                    self.text.remove(i);
                }
            }
            KeyCode::Char(c) if !ctrl && !alt => {
                self.cut();
                let i = self.byte(self.at);
                self.text.insert(i, c);
                self.at += 1;
            }
            _ => {}
        }
    }
}

/// Save `bytes`, an attachment named `name`, in `dir`, made if it is not
/// there, under a name not yet taken there; and where it was saved.
pub(super) fn save_one(
    dir: &std::path::Path,
    name: &str,
    bytes: &[u8],
) -> std::io::Result<PathBuf> {
    let path = unused(dir, &safe_name(name));
    std::fs::create_dir_all(dir).and_then(|_| write_new(&path, bytes))?;
    Ok(path)
}

impl App {
    /// List the attachments of the message read, to save.
    pub(super) fn list_to_save(&mut self) {
        let Some(m) = self.message() else {
            self.status = "nothing to save: ⏎ reads the message picked".to_string();
            return;
        };
        if m.attachments.is_empty() {
            self.status = "no attachments to save".to_string();
            return;
        }
        let items = m
            .attachments
            .iter()
            .enumerate()
            .map(|(at, a)| Item::File {
                at,
                name: a.name.clone(),
                mime: a.mime.clone(),
                size: a.size,
            })
            .collect();
        self.dialog = Dialog::Files(Files {
            items,
            at: 0,
            note: None,
            saving: true,
        });
    }

    /// A key in the files listed to save.
    pub(super) fn save_list_key(&mut self, key: KeyEvent) {
        match (keymap::act(key), key.code) {
            (Some(Act::Down), _) => self.dialog.step(1),
            (Some(Act::Up), _) => self.dialog.step(-1),
            (Some(Act::Open), _) | (_, KeyCode::Char('s')) => self.pick_save_dir(false),
            (_, KeyCode::Char('a' | 'S')) => self.pick_save_dir(true),
            _ => self.dialog = Dialog::Closed,
        }
    }

    /// Pick the folder to save in, the files listed put by till it is:
    /// begun where the last was saved, else in the downloads folder.
    pub(super) fn pick_save_dir(&mut self, all: bool) {
        self.save_dir_of_listed(all, true);
    }

    /// Pick the folder to save in, the files listed put by, and shown
    /// again after if they were `listed`.
    fn save_dir_of_listed(&mut self, all: bool, listed: bool) {
        let Dialog::Files(files) = std::mem::take(&mut self.dialog) else {
            return;
        };
        let dir = start_dir(self.save_dir.clone());
        self.dialog = Dialog::SaveTo(Box::new(SaveTo {
            picker: crate::pick::Picker::folder(dir),
            files,
            all,
            listed,
            naming: None,
        }));
    }

    /// Pick the folder to save all the message's attachments in, straight
    /// from the message: let go, back to it.
    pub(super) fn save_all(&mut self) {
        self.list_to_save();
        self.save_dir_of_listed(true, false);
    }

    /// A key while the folder to save in is picked, or the name to save
    /// under typed: esc from the name back to the folders.
    pub(super) fn save_to_key(&mut self, key: KeyEvent) {
        let Dialog::SaveTo(s) = &mut self.dialog else {
            return;
        };
        match &mut s.naming {
            Some(_) if key.code == KeyCode::Esc => s.naming = None,
            Some((dir, name)) if key.code == KeyCode::Enter => {
                let (dir, name) = (dir.clone(), name.text.trim().to_string());
                self.save_named(dir, name);
            }
            Some((_, name)) => name.key(key),
            None => {
                let picked = s.picker.key(key);
                self.saved_to(picked);
            }
        }
    }

    /// What came of picking the folder to save in: the name of the one
    /// picked asked; or all of them saved there, and closed; or, let go,
    /// the files listed again, if they were shown.
    pub(super) fn saved_to(&mut self, picked: crate::pick::Picked) {
        use crate::pick::Picked;
        let dir = match picked {
            Picked::Stay | Picked::Take(_) => return,
            Picked::Cancel => None,
            Picked::Folder(dir) => Some(dir),
        };
        let Dialog::SaveTo(s) = &mut self.dialog else {
            return;
        };
        if let Some(dir) = dir.clone()
            && !s.all
        {
            if let Some(Item::File { name, .. }) = s.files.items.get(s.files.at) {
                s.naming = Some((dir, Name::new(&safe_name(name))));
            }
            return;
        }
        let Dialog::SaveTo(s) = std::mem::take(&mut self.dialog) else {
            return;
        };
        match dir {
            Some(dir) => {
                if self.save_attachments(&dir) {
                    self.save_dir = Some(dir);
                }
            }
            None if s.listed => self.dialog = Dialog::Files(s.files),
            None => {}
        }
    }

    /// Save the one picked in `dir` under `name`, its own if none is
    /// typed; the files listed again, saying how it went.
    fn save_named(&mut self, dir: PathBuf, name: String) {
        let Dialog::SaveTo(s) = std::mem::take(&mut self.dialog) else {
            return;
        };
        self.dialog = Dialog::Files(s.files);
        // Begun in next only once something was saved there.
        if self.save_item(&dir, &name) {
            self.save_dir = Some(dir);
        }
    }

    /// Save the attachment picked in `dir`, under `name` if one is given;
    /// whether it was.
    fn save_item(&mut self, dir: &std::path::Path, name: &str) -> bool {
        let m = self.message().cloned();
        let (Some(files), Some(m)) = (self.dialog.files_mut(), m) else {
            return false;
        };
        let Some(Item::File { at, name: own, .. }) = files.items.get(files.at).cloned() else {
            return false;
        };
        let Some((_, bytes)) = mail::attachments(&m.raw).into_iter().nth(at) else {
            return false;
        };
        let name = if name.is_empty() {
            own
        } else {
            name.to_string()
        };
        let saved = save_one(dir, &name, &bytes);
        files.note = Some(match &saved {
            Ok(path) => format!("saved {}", tilde(path)),
            Err(e) => format!("cannot save {name}: {e}"),
        });
        saved.is_ok()
    }

    /// Save the message's attachments in `dir`; whether any were.
    fn save_attachments(&mut self, dir: &std::path::Path) -> bool {
        let Some(m) = self.message().cloned() else {
            return false;
        };
        let mut saved = Vec::new();
        for (Attachment { name, .. }, bytes) in mail::attachments(&m.raw) {
            match save_one(dir, &name, &bytes) {
                Ok(path) => saved.push(
                    path.file_name()
                        .map(|n| n.to_string_lossy().into_owned())
                        .unwrap_or_default(),
                ),
                Err(e) => {
                    self.status = format!("cannot save {name}: {e}");
                    return !saved.is_empty();
                }
            }
        }
        self.status = format!("saved to {}: {}", tilde(dir), saved.join(", "));
        true
    }
}
