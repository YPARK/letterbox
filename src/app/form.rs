//! The form an account is added with: what is typed in each field, filled
//! in from the address where it can be, and signing in to add it.
use super::*;

/// The fields of the form an account is added with, in their order; where
/// its password is kept is picked rather than typed, and the vault's
/// passphrase asked for only if it is kept there and the vault is locked,
/// twice if there is no vault yet. Where mail is sent is shown after the
/// user, though it is kept last.
pub const FIELDS: [&str; 11] = [
    "Email",
    "Name",
    "Server",
    "Port",
    "User",
    "Sign in",
    "Password",
    "Keep in",
    "Passphrase",
    "Again",
    "Outgoing",
];

pub(crate) const EMAIL: usize = 0;

pub(crate) const NAME: usize = 1;

pub(crate) const SERVER: usize = 2;

pub(super) const PORT: usize = 3;

pub(super) const USER: usize = 4;

pub const SIGN_IN: usize = 5;

pub const PASSWORD: usize = 6;

pub const KEEP: usize = 7;

pub const PASSPHRASE: usize = 8;

pub const AGAIN: usize = 9;

pub const OUTGOING: usize = 10;

/// Why the form could not add its account: a mistake in what is typed, or
/// the server refused to sign in.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Failure {
    Mistake,
    Refused,
}

/// An account being added: what is typed, the field typed in, and which
/// fields were typed rather than filled in from the address.
pub struct Form {
    pub fields: [String; 11],
    pub at: usize,
    pub(super) typed: [bool; 11],
    /// How it signs in: with a password, or on Microsoft's page.
    pub auth: Auth,
    /// The address Microsoft was last asked about.
    pub(super) asked: Option<String>,
    /// Where its secret may be kept, and where it will be.
    pub keepers: Vec<Keeper>,
    pub keeper: usize,
    /// What to know of the provider, or why signing in failed.
    pub note: Option<String>,
    pub failed: Option<Failure>,
    /// Whether it is signing in, to be added if it can; how to sign in on
    /// Microsoft's pages, while it waits; and what stops it waiting.
    pub checking: bool,
    pub waiting: Option<oauth::Waiting>,
    pub(super) stop: Arc<AtomicBool>,
}

impl Form {
    pub(super) fn new(keepers: Vec<Keeper>) -> Self {
        let mut fields: [String; 11] = Default::default();
        fields[PORT] = "993".to_string();
        Form {
            fields,
            at: EMAIL,
            typed: [false; 11],
            auth: Auth::Password,
            asked: None,
            keepers,
            keeper: 0,
            note: None,
            failed: None,
            checking: false,
            waiting: None,
            stop: Arc::new(AtomicBool::new(false)),
        }
    }

    pub fn kept(&self) -> Option<Keeper> {
        self.keepers.get(self.keeper).copied()
    }

    /// The fields shown, by their place: the password only if it signs in
    /// with one; the vault's passphrase only if it is to be opened, or made.
    pub fn shown(&self) -> Vec<usize> {
        let mut all: Vec<usize> = (EMAIL..=USER).collect();
        all.extend([OUTGOING, SIGN_IN]);
        if self.auth == Auth::Password {
            all.push(PASSWORD);
        }
        all.push(KEEP);
        if self.kept() == Some(Keeper::Vault) && !vault::is_open() {
            all.push(PASSPHRASE);
            if !vault::exists() {
                all.push(AGAIN);
            }
        }
        all
    }

    /// Whether ⏎ in the field typed in adds the account: past what must be
    /// typed for it.
    pub(super) fn last(&self) -> bool {
        self.at >= PASSWORD || (self.at == SIGN_IN && self.auth == Auth::Microsoft)
    }

    /// Go to the field `by` after the one typed in, among those shown.
    pub(super) fn step(&mut self, by: isize) {
        let shown = self.shown();
        let at = shown.iter().position(|&i| i == self.at).unwrap_or(0) as isize;
        let n = shown.len() as isize;
        self.at = shown[(at + by).rem_euclid(n) as usize];
    }

    /// Sign in the other way.
    pub(super) fn flip(&mut self) {
        self.auth = match self.auth {
            Auth::Password => Auth::Microsoft,
            Auth::Microsoft => Auth::Password,
        };
        self.typed[SIGN_IN] = true;
        if self.auth == Auth::Microsoft && !self.typed[SERVER] {
            self.fields[SERVER] = oauth::IMAP_HOST.to_string();
        }
    }

    /// Sign in with Microsoft, as Microsoft says the address does, where the
    /// way to sign in was not picked.
    pub(super) fn microsoft(&mut self) {
        if self.typed[SIGN_IN] {
            return;
        }
        self.auth = Auth::Microsoft;
        if !self.typed[SERVER] {
            self.fields[SERVER] = oauth::IMAP_HOST.to_string();
        }
        self.outgoing();
        if self.failed.is_none() {
            self.note = Some(config::MICROSOFT_NOTE.to_string());
        }
    }

    /// Where mail is sent, as the address, the server and the way it signs
    /// in say, where it was not typed.
    pub(super) fn outgoing(&mut self) {
        if !self.typed[OUTGOING] {
            self.fields[OUTGOING] =
                config::guess_outgoing(&self.fields[EMAIL], &self.fields[SERVER], self.auth);
        }
    }

    /// Keep the password in the next place it may be kept, or the one before.
    pub(super) fn turn(&mut self, by: isize) {
        let n = self.keepers.len().max(1) as isize;
        self.keeper = (self.keeper as isize + by).rem_euclid(n) as usize;
    }

    /// Fill in what the address tells, where nothing was typed.
    pub(super) fn fill(&mut self) {
        let email = self.fields[EMAIL].trim().to_string();
        let (host, note) = config::guess(&email);
        let microsoft = host == oauth::IMAP_HOST;
        if !self.typed[NAME] {
            self.fields[NAME] = setup::name_for(&email);
        }
        if !self.typed[SERVER] {
            self.fields[SERVER] = host;
        }
        if !self.typed[USER] {
            self.fields[USER] = email;
        }
        if self.failed.is_none() {
            self.note = note.map(str::to_string);
        }
        if !self.typed[SIGN_IN] {
            self.auth = if microsoft {
                Auth::Microsoft
            } else {
                Auth::Password
            };
        }
    }
}

impl App {
    /// Open the form to add an account.
    pub(super) fn open_form(&mut self) {
        if !self.editable {
            self.status =
                "the demo's account is made up: run letterbox without --demo to add yours"
                    .to_string();
            return;
        }
        self.searching = false;
        let keepers = (self.keepers)();
        self.dialog = Dialog::Form(Box::new(Form::new(keepers)));
    }

    /// Sign in with what the form holds, on a thread of its own; the
    /// account is added if it can.
    pub(super) fn submit(&mut self) {
        let Some(form) = self.dialog.form_mut() else {
            return;
        };
        let f = &form.fields;
        let microsoft = form.auth == Auth::Microsoft;
        let missing = [EMAIL, NAME, SERVER, PASSWORD]
            .into_iter()
            .filter(|&i| i != PASSWORD || !microsoft)
            .find(|&i| f[i].trim().is_empty());
        let port = f[PORT].trim().parse::<u16>();
        if let Some(i) = missing {
            form.at = i;
            form.failed = Some(Failure::Mistake);
            form.note = Some(format!("{} is needed", FIELDS[i]));
            return;
        }
        let Ok(port) = port else {
            form.at = PORT;
            form.failed = Some(Failure::Mistake);
            form.note = Some("the port is a number, as 993".to_string());
            return;
        };
        let Some((smtp_host, smtp_port)) = setup::outgoing(&f[OUTGOING]) else {
            form.at = OUTGOING;
            form.failed = Some(Failure::Mistake);
            form.note =
                Some("a port after the colon is a number, as smtp.example.com:465".to_string());
            return;
        };
        let Some(keeper) = form.kept() else {
            return;
        };
        let shown = form.shown();
        let passphrase = shown
            .contains(&PASSPHRASE)
            .then(|| form.fields[PASSPHRASE].clone());
        if let Some(p) = &passphrase {
            let problem = if p.is_empty() {
                Some((PASSPHRASE, "the vault's passphrase is needed"))
            } else if shown.contains(&AGAIN) && form.fields[AGAIN] != *p {
                Some((AGAIN, "the two passphrases differ"))
            } else if shown.contains(&AGAIN) && p.chars().count() < 8 {
                Some((PASSPHRASE, "a vault's passphrase is 8 characters or more"))
            } else {
                None
            };
            if let Some((at, why)) = problem {
                form.at = at;
                form.failed = Some(Failure::Mistake);
                form.note = Some(why.to_string());
                return;
            }
        }
        let mut account = setup::account(&f[NAME], &f[EMAIL], &f[SERVER], port, &f[USER]);
        account.auth = form.auth;
        account.smtp_host = smtp_host;
        account.smtp_port = smtp_port;
        if !microsoft {
            account.password = Some(f[PASSWORD].clone());
        }
        form.checking = true;
        form.failed = None;
        form.waiting = None;
        form.note = Some(if microsoft {
            "asking Microsoft…".to_string()
        } else {
            format!("signing in to {}…", account.host)
        });
        let stop = Arc::new(AtomicBool::new(false));
        form.stop = stop.clone();
        let (path, keep, replies) = (self.config.clone(), self.keep, self.replies_to.clone());
        let browse = self.browse;
        self.pending += 1;
        std::thread::spawn(move || {
            let name = account.name.clone();
            let shown = replies.clone();
            let done = passphrase
                .map_or(Ok(()), |p| vault::unlock(&p))
                .and_then(|()| {
                    if microsoft {
                        let email = account.email.clone();
                        let tokens = oauth::sign_in(
                            &email,
                            account.tenant(),
                            account.client(),
                            &stop,
                            browse,
                            |waiting| {
                                let _ = shown.send(Reply {
                                    account: usize::MAX,
                                    answer: Answer::Waiting(waiting.clone()),
                                });
                            },
                        )?;
                        account.password = Some(tokens.refresh.clone());
                        oauth::remember(&account, tokens);
                    }
                    setup::add(&path, account, keeper, keep)
                })
                .map(|(imap, folders)| Added {
                    name,
                    store: Box::new(imap),
                    folders,
                })
                .map_err(worker::text);
            let _ = replies.send(Reply {
                account: usize::MAX,
                answer: Answer::Added(done),
            });
        });
    }

    /// Ask Microsoft, on a thread of its own, whether the address in the
    /// form signs in with it, unless it was asked already.
    pub(super) fn ask_microsoft(&mut self) {
        let Some(form) = self.dialog.form_mut() else {
            return;
        };
        let email = form.fields[EMAIL].trim().to_string();
        if !email.contains('@') || form.asked.as_ref() == Some(&email) {
            return;
        }
        form.asked = Some(email.clone());
        let replies = self.replies_to.clone();
        self.pending += 1;
        std::thread::spawn(move || {
            let microsoft = oauth::is_microsoft(&email);
            let _ = replies.send(Reply {
                account: usize::MAX,
                answer: Answer::Realm { email, microsoft },
            });
        });
    }

    /// Take on the account the form added, and list its inbox; or say in
    /// the form why it could not be.
    pub(super) fn added(&mut self, done: Result<Added, String>) {
        match done {
            Ok(Added {
                name,
                store,
                folders,
            }) => {
                if self.dialog.form().is_some() {
                    self.dialog = Dialog::Closed;
                }
                let a = self.join(name.clone(), store);
                self.accounts[a].folders = folders;
                self.accounts[a].state = State::Ready;
                self.rebuild();
                if let Some(row) = self
                    .rows
                    .iter()
                    .position(|r| matches!(r, Row::Folder(x, _) if *x == a))
                {
                    self.row = row;
                    self.open_folder();
                    self.focus = Focus::List;
                }
                let paths: Vec<String> = self.accounts[a]
                    .folders
                    .iter()
                    .filter(|f| f.selectable)
                    .map(|f| f.path.clone())
                    .collect();
                for p in paths {
                    self.ask(a, Ask::Count(p));
                }
                self.status = format!("added {name}");
            }
            Err(e) => match self.dialog.form_mut() {
                Some(form) => {
                    form.checking = false;
                    form.failed = Some(Failure::Refused);
                    form.note = Some(e);
                }
                None => self.status = format!("cannot add the account: {}", first_line(&e)),
            },
        }
    }

    /// Remove the account at `a`: from the file, its password forgotten,
    /// and from the screen.
    pub(super) fn remove(&mut self, a: usize) {
        let name = self.accounts[a].name.clone();
        let forgotten = match setup::remove(&self.config, &name, false) {
            Ok((_, forgotten)) => forgotten,
            Err(e) => {
                self.status = format!("cannot remove {name}: {e:#}");
                return;
            }
        };
        self.accounts[a].asks = None;
        if let Some((_, stop)) = self.syncs.get_mut(a).and_then(Option::take) {
            stop.store(true, Ordering::Relaxed);
        }
        self.listed.retain(|(x, _), _| *x != a);
        self.read.forget(a);
        // Folders together are no more with one account left.
        let alone = self.live().filter(|&x| x != a).count() < 2;
        if self.listing.as_ref().is_some_and(|l| {
            l.parts.iter().any(|p| p.account == a) || (alone && l.account().is_none())
        }) {
            self.listing = None;
            self.reading = None;
            self.shown.clear();
            self.query.clear();
            self.found = None;
        }
        self.rebuild();
        if self.listing.is_none() {
            self.open_folder();
        }
        self.status = if forgotten {
            format!("removed {name}; its mail stays on its server")
        } else {
            format!("removed {name}; its password stays in the vault, which is locked")
        };
    }

    /// Take a key in the form.
    pub(super) fn form_key(&mut self, key: KeyEvent) {
        let ctrl = key.modifiers.contains(KeyModifiers::CONTROL);
        let browse = self.browse;
        let Some(form) = self.dialog.form_mut() else {
            return;
        };
        if form.checking {
            if key.code == KeyCode::Esc {
                form.stop.store(true, Ordering::Relaxed);
                self.dialog = Dialog::Closed;
                self.status = "stopped adding the account".into();
            }
            return;
        }
        let before = form.at;
        match key.code {
            KeyCode::Esc => self.dialog = Dialog::Closed,
            KeyCode::Tab | KeyCode::Down => form.step(1),
            KeyCode::BackTab | KeyCode::Up => form.step(-1),
            KeyCode::Enter if form.last() => self.submit(),
            KeyCode::Enter => form.step(1),
            KeyCode::Char('o') if ctrl => {
                if let Some((_, page)) =
                    config::app_password_help(&form.fields[EMAIL], &form.fields[SERVER])
                {
                    browse(page);
                    self.status = "opened in the browser".to_string();
                }
            }
            KeyCode::Left if form.at == KEEP => form.turn(-1),
            KeyCode::Right | KeyCode::Char(' ') if form.at == KEEP => form.turn(1),
            KeyCode::Left | KeyCode::Right | KeyCode::Char(' ') if form.at == SIGN_IN => {
                form.flip()
            }
            _ if form.at == KEEP || form.at == SIGN_IN => {}
            _ => {
                if edit_line(&mut form.fields[form.at], key) {
                    form.typed[form.at] = true;
                }
            }
        }
        self.form_changed(before);
    }

    /// Typed into the form's field, as one: its lines run together, as a
    /// password copied ends with one.
    pub(super) fn form_paste(&mut self, text: &str) {
        let Some(form) = self.dialog.form_mut() else {
            return;
        };
        if form.checking || form.at == KEEP || form.at == SIGN_IN {
            return;
        }
        let text: String = text.chars().filter(|c| !matches!(c, '\r' | '\n')).collect();
        if text.is_empty() {
            return;
        }
        form.fields[form.at].push_str(&text);
        form.typed[form.at] = true;
        let at = form.at;
        self.form_changed(at);
    }

    /// What follows from the form changed, from the field `before` it.
    pub(super) fn form_changed(&mut self, before: usize) {
        let Some(form) = self.dialog.form_mut() else {
            return;
        };
        if form.at == EMAIL && !form.checking {
            form.failed = None;
            form.fill();
        }
        if !form.checking {
            form.outgoing();
        }
        if before == EMAIL && form.at != EMAIL {
            self.ask_microsoft();
        }
    }
}
