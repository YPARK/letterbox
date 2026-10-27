//! The message being written: new, a reply or a forward, its files
//! attached, and sent.
use super::*;

/// A message being written, in the pane that covers the list and the
/// message read.
pub struct Compose {
    pub buffer: edit::Buffer,
    pub account: usize,
    /// What it answers, if anything.
    pub(super) thread: Thread,
    /// `C-c` was pressed, and the next key sends or cancels.
    pub prefix: bool,
    pub scroll: usize,
    /// A send has been asked and not yet answered; the pane stays up.
    pub sending: bool,
    /// The files of the message it forwards, sent with it while an
    /// `Attach` line names them.
    pub files: Vec<compose::File>,
    /// Said that what is attached is more than most servers take: a send
    /// asked again sends it.
    pub warned: bool,
    /// The text it was opened with, to tell whether it has been changed.
    pub(super) original: String,
    /// `C-c C-k` was pressed on a message changed: whether to let it go is
    /// asked, as Emacs asks.
    pub asking: bool,
}

/// What `C-c C-k` asks of a message changed, as Emacs's message-mode asks,
/// and what each answer does.
pub const KILL_ANYWAY: &str =
    "Message modified; kill anyway? (y or n) · y discards it, n goes on writing";

/// What `C-g` says when it had nothing to let go: that the message stays,
/// and how it is sent or let go.
pub(super) const QUIT: &str = "Quit · the message stays: C-c C-c sends it, C-c C-k discards it";

/// How much attached most servers take, in bytes.
pub(super) const LARGE: usize = 25 * 1024 * 1024;

/// A message to answer once it is read: where it is, and how.
pub(super) struct Pending {
    pub(super) account: usize,
    pub(super) folder: String,
    pub(super) uid: u32,
    pub(super) how: Respond,
}

impl App {
    pub(super) fn compose_new(&mut self) {
        let Some(account) = self.account_of() else {
            self.status = "no account to send from".to_string();
            return;
        };
        let (text, row, col) = compose::blank();
        self.open_compose(account, text, (row, col), Thread::default(), Vec::new());
    }

    /// Answer the message picked, as `how` says; read first, if it was
    /// not.
    pub(super) fn respond(&mut self, how: Respond) {
        let Some(summary) = self.picked() else {
            self.status = "no message to answer".to_string();
            return;
        };
        let (account, folder, uid) = (summary.account, summary.folder, summary.summary.uid);
        // Answered, it is kept: learnt as not thrown away.
        self.teach_at(account, &folder, uid, false);
        if let Some(message) = self.cached(account, &folder, uid) {
            self.open_reply(account, &folder, &message, how);
            return;
        }
        self.reply_when = Some(Pending {
            account,
            folder: folder.clone(),
            uid,
            how,
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

    pub(super) fn cached(
        &self,
        account: usize,
        folder: &str,
        uid: u32,
    ) -> Option<std::sync::Arc<Message>> {
        if self
            .reading
            .as_ref()
            .is_some_and(|r| r.account == account && r.folder == folder && r.uid == uid)
            && let Some(message) = self.message()
        {
            return Some(message.clone());
        }
        self.read.get(&(account, folder.to_string(), uid))
    }

    /// Write the answer to `message`, in `folder` of `account`.
    pub(super) fn open_reply(
        &mut self,
        account: usize,
        folder: &str,
        message: &Message,
        how: Respond,
    ) {
        let me = self.accounts[account].email.clone();
        let (text, at, thread, files) = compose::respond(message, folder, how, &me);
        self.open_compose(account, text, at, thread, files);
    }

    pub(super) fn open_compose(
        &mut self,
        account: usize,
        text: String,
        (row, col): (usize, usize),
        thread: Thread,
        files: Vec<compose::File>,
    ) {
        let mut buffer = edit::Buffer::from_text(&text);
        buffer.place(row, col);
        let original = buffer.text();
        if matches!(self.dialog, Dialog::Help(_)) {
            self.dialog = Dialog::Closed;
        }
        self.searching = false;
        self.compose = Some(Compose {
            buffer,
            account,
            thread,
            prefix: false,
            scroll: 0,
            sending: false,
            files,
            warned: false,
            original,
            asking: false,
        });
    }

    /// The account to write from: the one listed, else the one of the
    /// message picked, else the first.
    pub(super) fn account_of(&self) -> Option<usize> {
        self.listing
            .as_ref()
            .and_then(Listing::account)
            .or_else(|| self.current().map(|m| m.account))
            .or_else(|| self.live().next())
    }

    pub(super) fn compose_key(&mut self, key: KeyEvent) {
        // The model's changes gone through: what is a key of theirs, theirs.
        if self.checking.is_some() && self.checking_key(key) {
            return;
        }
        let Some(compose) = &mut self.compose else {
            return;
        };
        if compose.sending {
            return;
        }
        if compose.asking {
            compose.asking = false;
            if matches!(key.code, KeyCode::Char('y' | 'Y')) && key.modifiers.is_empty() {
                self.compose = None;
                self.status = "discarded, not sent".to_string();
            } else {
                self.status = "kept · C-c C-c sends it".to_string();
            }
            return;
        }
        let done = edit::handle(&mut compose.buffer, &mut compose.prefix, key);
        match done {
            Done::Stay => {}
            Done::Quit => self.status = QUIT.to_string(),
            Done::Cancel if compose.buffer.text() != compose.original => {
                compose.asking = true;
                self.status = KILL_ANYWAY.to_string();
            }
            Done::Cancel => {
                self.compose = None;
                self.status = "not sent".to_string();
            }
            Done::Send => self.send_compose(),
            Done::Check => self.check_grammar(),
            Done::Attach => {
                let dir = self
                    .attach_dir
                    .clone()
                    .filter(|d| d.is_dir())
                    .or_else(|| Some(downloads()).filter(|d| d.is_dir()))
                    .unwrap_or_else(|| PathBuf::from(std::env::var_os("HOME").unwrap_or_default()));
                self.dialog = Dialog::Attach(Box::new(crate::pick::Picker::open(dir)));
            }
        }
    }

    /// A key while files to attach are picked.
    pub(super) fn attach_key(&mut self, key: KeyEvent) {
        let Dialog::Attach(picker) = &mut self.dialog else {
            return;
        };
        match picker.key(key) {
            crate::pick::Picked::Stay => {}
            crate::pick::Picked::Cancel => self.dialog = Dialog::Closed,
            crate::pick::Picked::Take(paths) => {
                self.attach_dir = Some(picker.dir.clone());
                self.dialog = Dialog::Closed;
                self.attach(&paths);
            }
        }
    }

    /// An `Attach` line for each of `paths` in the message being written,
    /// under its other headers; those it has already, once.
    pub(super) fn attach(&mut self, paths: &[PathBuf]) {
        let Some(compose) = &mut self.compose else {
            return;
        };
        let lines = compose.buffer.lines();
        let end = lines
            .iter()
            .position(String::is_empty)
            .unwrap_or(lines.len());
        let had: Vec<&str> = lines[..end]
            .iter()
            .filter_map(|l| l.strip_prefix("Attach:").map(str::trim))
            .collect();
        let new: Vec<String> = paths
            .iter()
            .map(|p| compose::named(p))
            .filter(|n| !had.contains(&n.as_str()))
            .collect();
        let lines: Vec<String> = new.iter().map(|n| format!("Attach: {n}")).collect();
        compose.buffer.insert_lines(end, &lines);
        self.status = match new.len() {
            0 => "attached already".to_string(),
            1 => format!("attached {}", new[0]),
            n => format!("attached {n} files"),
        };
    }

    /// What was pasted, or dropped onto the terminal as the paths of its
    /// files: attached, if the message being written takes them and each
    /// is a file; else written in, or typed into the search or the form.
    pub fn paste(&mut self, text: &str) {
        if self.dialog.form().is_some() {
            self.form_paste(text);
        } else if self.compose.is_some() && !self.dialog.is_open() {
            let paths = dropped(text);
            if !paths.is_empty() {
                return self.attach(&paths);
            }
            if let Some(compose) = &mut self.compose
                && !compose.sending
            {
                compose.buffer.paste(text);
            }
        } else if self.searching {
            self.query.push_str(&text.replace(['\r', '\n'], " "));
            self.found = None;
            self.narrow();
        }
    }

    pub(super) fn send_compose(&mut self) {
        let Some(compose) = &self.compose else {
            return;
        };
        if compose.sending {
            return;
        }
        let account = compose.account;
        let draft = match compose::parse(&compose.buffer.text()) {
            Ok(draft) => draft,
            Err(e) => {
                self.status = e;
                return;
            }
        };
        let email = self
            .accounts
            .get(account)
            .map(|a| a.email.clone())
            .unwrap_or_default();
        if email.is_empty() {
            self.status = "this account has no address to send from".to_string();
            return;
        }
        // Each file attached, read now: a forward's, or this machine's.
        let mut files = Vec::new();
        for value in &draft.attach {
            let file = match value.strip_suffix(compose::FORWARDED) {
                Some(name) => compose
                    .files
                    .iter()
                    .find(|f| f.name == name)
                    .cloned()
                    .ok_or_else(|| format!("{name} is not among the files forwarded")),
                None => compose::read_file(&compose::path_of(value))
                    .map_err(|e| format!("cannot attach {value}: {e}")),
            };
            match file {
                Ok(f) => files.push(f),
                Err(e) => {
                    self.status = e;
                    return;
                }
            }
        }
        let total: usize = files.iter().map(|f| f.bytes.len()).sum();
        if total > LARGE && !compose.warned {
            self.status = format!(
                "{} attached is more than most servers take, {}: C-c C-c again sends it",
                crate::ui::size(total),
                crate::ui::size(LARGE)
            );
            self.compose.as_mut().expect("open").warned = true;
            return;
        }
        let raw = compose::bytes(
            &email,
            &draft,
            compose.thread.in_reply_to.as_deref(),
            compose.thread.references.as_deref(),
            &files,
        );
        let answer = compose.thread.answering.clone();
        self.compose.as_mut().expect("open").sending = true;
        self.status = "sending…".to_string();
        self.ask(account, Ask::Send { raw, answer });
    }
}
