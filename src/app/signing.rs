//! A PDF attached, signed in ohp, and answered with: ohp given a file of
//! its own to say which signed copy it kept, which it keeps beside the PDF,
//! in the message's own folder, under the name it is given; once it quits,
//! a reply to the message, the copy attached, offered, or the copy saved
//! in a folder picked.
use super::*;
use std::sync::atomic::{AtomicU64, Ordering};

/// A PDF attached, presented in ohp, which may sign it: the message it
/// came with, and where ohp says which signed copy it kept, this
/// presenting's alone.
#[derive(Clone, Debug, PartialEq)]
pub struct Presenting {
    pub account: usize,
    pub folder: String,
    pub uid: u32,
    pub report: PathBuf,
}

/// A copy of a PDF attached, signed in ohp: where it is, the message to
/// answer with it, the files listed it was opened from, to go back to,
/// the folder to save it in, as it is picked, and how saving it went.
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
    /// A PDF attached to `uid` in `folder` of `account`, to be presented:
    /// its report a file not yet there, this letterbox's own.
    pub fn new(account: usize, folder: String, uid: u32) -> Presenting {
        let n = REPORTS.fetch_add(1, Ordering::Relaxed);
        let report = cache()
            .join("reports")
            .join(format!("{}-{n}", std::process::id()));
        Presenting {
            account,
            folder,
            uid,
            report,
        }
    }

    /// What ohp is told: where to say which signed copy it kept.
    pub fn env(&self) -> [(&'static str, &std::ffi::OsStr); 1] {
        [("OHP_SIGNED", self.report.as_os_str())]
    }
}

impl App {
    /// ohp quit, having presented a PDF attached: the copy it signed, if
    /// it signed one, offered to reply with, over the files listed.
    pub fn presented(&mut self, from: &Presenting) {
        let said = std::fs::read_to_string(&from.report);
        let _ = std::fs::remove_file(&from.report);
        let Ok(said) = said else {
            return;
        };
        let path = PathBuf::from(said.trim());
        if !path.is_file() {
            return;
        }
        let files = match std::mem::take(&mut self.dialog) {
            Dialog::Files(files) => Some(files),
            _ => None,
        };
        self.dialog = Dialog::Signed(Box::new(Signed {
            path,
            from: from.clone(),
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
                    let saved =
                        std::fs::read(&signed.path).and_then(|bytes| save_one(&dir, &name, &bytes));
                    signed.note = Some(match &saved {
                        Ok(to) => format!("saved {}", tilde(to)),
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
            unreachable!()
        };
        let Signed {
            path, from, files, ..
        } = *signed;
        let how = match key.code {
            KeyCode::Enter | KeyCode::Char('r') => Respond::Reply,
            KeyCode::Char('a') => Respond::All,
            _ => {
                self.status = format!("signed copy kept: {} · C-c C-a attaches it", tilde(&path));
                if let Some(files) = files {
                    self.dialog = Dialog::Files(files);
                }
                return;
            }
        };
        self.respond_to(from.account, from.folder, from.uid, how, Some(path));
    }
}
