//! A PDF attached, signed in ohp, and answered with: ohp told the file's
//! own name, and a file of its own to say which signed copy it kept in;
//! once it quits, the copy kept with the message it came with, and a reply
//! to that message, the copy attached, offered, or the copy saved in a
//! folder picked.
use super::*;
use std::sync::atomic::{AtomicU64, Ordering};

/// A PDF attached, presented in ohp, which may sign it: the message it
/// came with, the file's own name, and where ohp says which signed copy
/// it kept, this presenting's alone.
#[derive(Clone, Debug, PartialEq)]
pub struct Presenting {
    pub account: usize,
    pub folder: String,
    pub uid: u32,
    pub name: String,
    pub report: PathBuf,
}

/// A copy of a PDF attached, signed in ohp: where it is kept, the message
/// to answer with it, the files listed it was opened from, to go back to,
/// and the folder to save it in, as it is picked.
pub struct Signed {
    pub path: PathBuf,
    pub from: Presenting,
    pub files: Option<Files>,
    pub saving: Option<crate::pick::Picker>,
    pub note: Option<String>,
}

/// How many reports this letterbox has asked of ohp: each its own file.
static REPORTS: AtomicU64 = AtomicU64::new(0);

impl Presenting {
    /// The PDF `name`, attached to `uid` in `folder` of `account`, to be
    /// presented: its report a file not yet there, this letterbox's own.
    pub fn new(account: usize, folder: &str, uid: u32, name: &str) -> Presenting {
        let n = REPORTS.fetch_add(1, Ordering::Relaxed);
        let report = cache()
            .join("reports")
            .join(format!("{}-{n}", std::process::id()));
        Presenting {
            account,
            folder: folder.to_string(),
            uid,
            name: name.to_string(),
            report,
        }
    }

    /// What ohp is told: the file's own name, as letterbox keeps it under
    /// another, and where to say which signed copy it kept.
    pub fn env(&self) -> [(&'static str, std::ffi::OsString); 2] {
        [
            ("OHP_AS", safe_name(&self.name).into()),
            ("OHP_SIGNED", self.report.clone().into()),
        ]
    }

    /// Where the copies signed of this message's files are kept: apart
    /// from every other message's, a uid being its folder's alone.
    fn kept_in(&self) -> PathBuf {
        let folder: String = self
            .folder
            .chars()
            .map(|c| {
                if matches!(c, '/' | '\\' | ':') {
                    '_'
                } else {
                    c
                }
            })
            .collect();
        cache()
            .join("files")
            .join("signed")
            .join(format!("{}-{folder}-{}", self.account, self.uid))
    }
}

impl App {
    /// ohp quit, having presented a PDF attached: the copy it signed, if
    /// it signed one, copied to be kept with the message it came with, and
    /// a reply with it offered over the files listed, if they were.
    pub fn presented(&mut self, from: Presenting) {
        let said = std::fs::read_to_string(&from.report);
        let _ = std::fs::remove_file(&from.report);
        let Ok(said) = said else {
            return;
        };
        let copy = PathBuf::from(said.trim());
        let Some(name) = copy.file_name().filter(|_| copy.is_file()) else {
            return;
        };
        let dir = from.kept_in();
        let path = dir.join(name);
        // Copied, not moved: where ohp said it kept it, it stays, but for
        // letterbox's own copy of the file, beside which ohp wrote it.
        if let Err(e) = private_dir(&dir).and_then(|_| std::fs::copy(&copy, &path)) {
            self.status = format!("cannot keep the signed copy, {}: {e}", tilde(&copy));
            return;
        }
        if copy.parent() == Some(cache().join("files").as_path()) {
            let _ = std::fs::remove_file(&copy);
        }
        let files = match std::mem::take(&mut self.dialog) {
            Dialog::Files(files) => Some(files),
            other => {
                self.dialog = other;
                None
            }
        };
        self.dialog = Dialog::Signed(Box::new(Signed {
            path,
            from,
            files,
            saving: None,
            note: None,
        }));
    }

    /// A key in what is asked of a signed copy: reply with it, to the
    /// sender or to all, save it in a folder picked, or keep it only.
    pub(super) fn signed_key(&mut self, key: KeyEvent) {
        let Dialog::Signed(signed) = &mut self.dialog else {
            return;
        };
        if let Some(picker) = &mut signed.saving {
            use crate::pick::Picked;
            match picker.key(key) {
                Picked::Stay | Picked::Take(_) => {}
                Picked::Cancel => signed.saving = None,
                Picked::Folder(dir) => {
                    signed.saving = None;
                    let name = signed
                        .path
                        .file_name()
                        .unwrap_or_default()
                        .to_string_lossy();
                    let to = unused(&dir, &name);
                    let saved = std::fs::create_dir_all(&dir)
                        .and_then(|_| std::fs::copy(&signed.path, &to));
                    signed.note = Some(match &saved {
                        Ok(_) => format!("saved {}", tilde(&to)),
                        Err(e) => format!("cannot save it in {}: {e}", tilde(&dir)),
                    });
                    // Begun in next only once something was saved there.
                    if saved.is_ok() {
                        self.save_dir = Some(dir);
                    }
                }
            }
            return;
        }
        if key.code == KeyCode::Char('s') {
            let dir = start_dir(self.save_dir.clone());
            signed.saving = Some(crate::pick::Picker::folder(dir));
            return;
        }
        let Dialog::Signed(signed) = std::mem::take(&mut self.dialog) else {
            return;
        };
        match key.code {
            KeyCode::Enter | KeyCode::Char('r') => self.reply_with(*signed, Respond::Reply),
            KeyCode::Char('a') => self.reply_with(*signed, Respond::All),
            _ => {
                self.status = format!(
                    "signed copy kept: {} · C-c C-a attaches it",
                    tilde(&signed.path)
                );
                if let Some(files) = signed.files {
                    self.dialog = Dialog::Files(files);
                }
            }
        }
    }

    /// Answer the message `signed` came with, as `how` says, the signed
    /// copy attached: read again first, if it is no longer.
    fn reply_with(&mut self, signed: Signed, how: Respond) {
        let Presenting {
            account,
            folder,
            uid,
            ..
        } = signed.from;
        // Answered, it is kept: learnt as not thrown away.
        self.teach_at(account, &folder, uid, false);
        if let Some(message) = self.cached(account, &folder, uid) {
            self.open_reply(account, &folder, &message, how);
            self.attach(std::slice::from_ref(&signed.path));
            return;
        }
        self.reply_when = Some(Pending {
            account,
            folder: folder.clone(),
            uid,
            how,
            attach: Some(signed.path),
        });
        self.status = "reading it first".to_string();
        self.ask(
            account,
            Ask::Open {
                folder,
                uid,
                mark: false,
            },
        );
    }
}
