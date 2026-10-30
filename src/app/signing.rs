//! A PDF attached, signed in ohp, and answered with: ohp told the file's
//! own name, and where to say which signed copy it kept; once it quits,
//! the copy kept with the message it came with, and a reply to that
//! message, the copy attached, offered.
use super::*;

/// A PDF attached, presented in ohp, which may sign it: the message it
/// came with, and the file's own name.
#[derive(Clone, Debug, PartialEq)]
pub struct Presenting {
    pub account: usize,
    pub folder: String,
    pub uid: u32,
    pub name: String,
}

/// A copy of a PDF attached, signed in ohp: where it is kept, and the
/// message to answer with it.
#[derive(Clone, Debug, PartialEq)]
pub struct Signed {
    pub path: PathBuf,
    pub account: usize,
    pub folder: String,
    pub uid: u32,
}

/// Where ohp says which signed copy it kept.
fn report() -> PathBuf {
    cache().join("files").join("signed")
}

impl Presenting {
    /// What ohp is told: the file's own name, as letterbox keeps it under
    /// another, and where to say which signed copy it kept.
    pub fn env(&self) -> Vec<(&'static str, std::ffi::OsString)> {
        let _ = std::fs::remove_file(report());
        vec![
            ("OHP_AS", safe_name(&self.name).into()),
            ("OHP_SIGNED", report().into()),
        ]
    }
}

impl App {
    /// ohp quit, having presented a PDF attached: the copy it signed, if
    /// it signed one, kept with the message it came with, each message's
    /// apart, and a reply with it offered.
    pub fn presented(&mut self, presented: Presenting) {
        let Ok(said) = std::fs::read_to_string(report()) else {
            return;
        };
        let _ = std::fs::remove_file(report());
        let copy = PathBuf::from(said.trim());
        let Some(name) = copy.file_name().filter(|_| copy.is_file()) else {
            return;
        };
        let dir = cache()
            .join("files")
            .join("signed")
            .join(format!("{}-{}", presented.account, presented.uid));
        let path = dir.join(name);
        let kept = private_dir(&dir).and_then(|_| std::fs::rename(&copy, &path));
        if let Err(e) = kept {
            self.status = format!("cannot keep the signed copy, {}: {e}", tilde(&copy));
            return;
        }
        self.dialog = Dialog::Signed(Box::new(Signed {
            path,
            account: presented.account,
            folder: presented.folder,
            uid: presented.uid,
        }));
    }

    /// A key in what is asked of a signed copy: reply with it, to the
    /// sender or to all, or keep it only.
    pub(super) fn signed_key(&mut self, key: KeyEvent) {
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
                )
            }
        }
    }

    /// Answer the message `signed` came with, as `how` says, the signed
    /// copy attached.
    fn reply_with(&mut self, signed: Signed, how: Respond) {
        let Some(message) = self.cached(signed.account, &signed.folder, signed.uid) else {
            self.status = format!(
                "its message is no longer read: the signed copy is {}",
                tilde(&signed.path)
            );
            return;
        };
        // Answered, it is kept: learnt as not thrown away.
        self.teach_at(signed.account, &signed.folder, signed.uid, false);
        self.open_reply(signed.account, &signed.folder, &message, how);
        self.attach(std::slice::from_ref(&signed.path));
    }
}
