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
    /// What was said of it before it was sent, as a large file, no
    /// subject, or many it goes to: a send asked again, with the same
    /// said, sends it.
    pub warned: Option<String>,
    /// Sent, and held a while first: C-g takes it back.
    pub held: Option<Holding>,
    /// The text it was opened with, to tell whether it has been changed.
    pub(super) original: String,
    /// `C-c C-k`, `C-g` or `esc` was pressed on a message changed, or
    /// carrying files: whether to let it go is asked, as Emacs asks.
    pub asking: bool,
}

/// What `C-c C-k` and `C-g` ask of a message changed, or carrying files, as Emacs's message-mode asks,
/// and what each answer does.
pub const KILL_ANYWAY: &str =
    "Message modified; kill anyway? (y or n) · y discards it, n goes on writing";

/// How much attached most servers take, in bytes.
pub(super) const LARGE: usize = 25 * 1024 * 1024;

/// How long a message sent is held before it goes, to be taken back.
pub const SEND_HOLD: Duration = Duration::from_secs(10);

/// Over this many it goes to, a send is asked about once.
pub(super) const MANY: usize = 10;

/// A message sent, held until `until`: its bytes, and what it answers.
pub struct Holding {
    pub(super) until: Instant,
    raw: Vec<u8>,
    answer: Option<(String, u32)>,
    /// The seconds left, as last said.
    shown: u64,
}

/// What is said of a message held: how long until it goes, and how it is
/// taken back.
fn held_said(secs: u64) -> String {
    format!("sending in {secs} s · C-g takes it back, C-c C-c sends it now")
}

/// Whether what is written, not what it quotes or forwards, speaks of a
/// file attached.
fn speaks_of_files(body: &str) -> bool {
    body.lines()
        .take_while(|l| l.trim() != compose::FORWARD_LINE)
        .filter(|l| !l.trim_start().starts_with('>'))
        .any(|l| {
            let l = l.to_lowercase();
            ["attached", "attaching", "attachment", "enclosed"]
                .iter()
                .any(|w| l.contains(w))
        })
}

/// A message to answer once it is read: where it is, and how.
pub(super) struct Pending {
    pub(super) account: usize,
    pub(super) folder: String,
    pub(super) uid: u32,
    pub(super) how: Respond,
    /// A file to attach to it, once it is opened.
    pub(super) attach: Option<PathBuf>,
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
        self.respond_to(account, folder, uid, how, None);
    }

    /// Answer the message `uid` in `folder` of `account`, as `how` says,
    /// `attach` attached; read first, if it is no longer.
    pub(super) fn respond_to(
        &mut self,
        account: usize,
        folder: String,
        uid: u32,
        how: Respond,
        attach: Option<PathBuf>,
    ) {
        // Answered, it is kept: learnt as not thrown away.
        self.teach_at(account, &folder, uid, false);
        if let Some(message) = self.cached(account, &folder, uid) {
            self.open_reply(account, &folder, &message, how);
            if let Some(path) = attach {
                self.attach(&[path]);
            }
            return;
        }
        self.reply_when = Some(Pending {
            account,
            folder: folder.clone(),
            uid,
            how,
            attach,
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
            warned: None,
            held: None,
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
        // Asked if it is to go now, its grammar gone through: y sends it.
        if std::mem::take(&mut self.sending_asked) {
            if matches!(key.code, KeyCode::Char('y' | 'Y')) && key.modifiers.is_empty() {
                self.send_compose();
            } else {
                self.status = "kept · C-c C-c sends it".to_string();
            }
            return;
        }
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
        if compose.held.is_some() {
            return self.held_key(key);
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
        // Sent as it is only by the C-c C-c that let its checking go.
        if done == Done::Stay && !compose.prefix {
            self.send_as_is = false;
        }
        match done {
            Done::Stay => {}
            // Asked first if changed, or carrying files: a forward's, set
            // up, is not lost to a stray key.
            Done::Cancel
                if compose.buffer.text() != compose.original || !compose.files.is_empty() =>
            {
                compose.asking = true;
                self.status = KILL_ANYWAY.to_string();
            }
            Done::Cancel => {
                self.compose = None;
                self.status = "not sent".to_string();
            }
            Done::Send => self.send_or_check(),
            Done::Check => self.check_grammar(false),
            Done::Attach => {
                let dir = start_dir(self.attach_dir.clone());
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
            crate::pick::Picked::Stay | crate::pick::Picked::Folder(_) => {}
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
        if compose.held.is_some() {
            return self.send_held();
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
        // What is easily got wrong, said once: a send asked again, with
        // nothing more to say, sends it.
        let total: usize = files.iter().map(|f| f.bytes.len()).sum();
        let mut said = Vec::new();
        if total > LARGE {
            said.push(format!(
                "{} attached is more than most servers take, {}",
                crate::ui::size(total),
                crate::ui::size(LARGE)
            ));
        }
        if draft.subject.trim().is_empty() {
            said.push("it has no subject".to_string());
        }
        let many = draft.to.len() + draft.cc.len();
        if many > MANY {
            said.push(format!("it goes to {many} people"));
        }
        if files.is_empty() && speaks_of_files(&draft.body) {
            said.push("it speaks of a file attached, and none is".to_string());
        }
        let said = said.join(", ");
        if !said.is_empty() && compose.warned.as_deref() != Some(said.as_str()) {
            self.status = format!("{said}: C-c C-c again sends it");
            self.compose.as_mut().expect("open").warned = Some(said);
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
        let compose = self.compose.as_mut().expect("open");
        let secs = self.send_hold.as_secs().max(1);
        compose.held = Some(Holding {
            until: Instant::now() + self.send_hold,
            raw,
            answer,
            shown: secs,
        });
        if self.send_hold.is_zero() {
            return self.send_held();
        }
        self.status = held_said(secs);
    }

    /// Send the message held, now.
    pub(super) fn send_held(&mut self) {
        let Some(compose) = &mut self.compose else {
            return;
        };
        let Some(held) = compose.held.take() else {
            return;
        };
        let account = compose.account;
        compose.sending = true;
        compose.prefix = false;
        self.status = "sending…".to_string();
        self.ask(
            account,
            Ask::Send {
                raw: held.raw,
                answer: held.answer,
            },
        );
    }

    /// A key while the message sent is held: C-g takes it back, C-c C-c
    /// sends it now; nothing else changes it.
    fn held_key(&mut self, key: KeyEvent) {
        let Some(compose) = &mut self.compose else {
            return;
        };
        let ctrl = key.modifiers.contains(KeyModifiers::CONTROL);
        match key.code {
            KeyCode::Char('g') if ctrl => {
                compose.held = None;
                compose.prefix = false;
                self.status = "not sent: still yours to change · C-c C-c sends it".to_string();
            }
            KeyCode::Char('c') if ctrl && compose.prefix => self.send_held(),
            KeyCode::Char('c') if ctrl => compose.prefix = true,
            _ => {
                compose.prefix = false;
                let secs = compose.held.as_ref().map_or(0, |h| h.shown);
                self.status = held_said(secs);
            }
        }
    }

    /// The message held, sent once its time is up, and how long it has
    /// left said as it goes; whether anything changed.
    pub(super) fn tick_held(&mut self) -> bool {
        let Some(held) = self.compose.as_mut().and_then(|c| c.held.as_mut()) else {
            return false;
        };
        let left = held.until.saturating_duration_since(Instant::now());
        if left.is_zero() {
            self.send_held();
            return true;
        }
        let secs = left.as_secs() + u64::from(left.subsec_nanos() > 0);
        if secs == held.shown {
            return false;
        }
        held.shown = secs;
        self.status = held_said(secs);
        true
    }
}
