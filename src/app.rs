//! The TUI: what is shown, and what keys do. Each account's store is asked
//! from its own thread; what it answers is taken in between keys, so the
//! screen is never kept waiting by a server.

use crate::compose;
use crate::compose::{Respond, Thread};
use crate::config::{self, Auth};
use crate::edit::{self, Done};
use crate::keymap::{self, Act};
use crate::mail::{self, Attachment, Folder, Message, Role, Summary};
use crate::oauth;
use crate::secret::Keeper;
use crate::setup;
use crate::store::{Flag, Removal, Store};
use crate::theme::{self, Theme};
use crate::trash::{self, Entry};
use crate::ui;
use crate::vault;
use crate::worker::{self, Added, Answer, Ask, Reply};
use ratatui::DefaultTerminal;
use ratatui::crossterm::event::{
    self, DisableMouseCapture, EnableMouseCapture, Event, KeyCode, KeyEvent, KeyEventKind,
    KeyModifiers, MouseButton, MouseEvent, MouseEventKind,
};
use ratatui::crossterm::event::{
    KeyboardEnhancementFlags, PopKeyboardEnhancementFlags, PushKeyboardEnhancementFlags,
};
use ratatui::crossterm::execute;
use ratatui::layout::{Position, Rect};
use ratatui::text::Line;
use ratatui::widgets::ListState;
use std::collections::{HashMap, HashSet, VecDeque};
use std::path::PathBuf;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{self, Receiver, Sender};
use std::time::{Duration, Instant};

/// Messages listed at a time; more as the list is moved to its end.
pub const PAGE: usize = 100;
/// Messages a search finds, at most.
const FOUND: usize = 200;
/// How long a folder or message is passed over before it is fetched,
/// while moving through the list.
const PAUSE: Duration = Duration::from_millis(150);
/// How often the folder shown is looked in again, and the folders counted.
const REFRESH: Duration = Duration::from_secs(120);
const DOUBLE_CLICK: Duration = Duration::from_millis(400);
/// How often answers are looked for, between keys.
const POLL: Duration = Duration::from_millis(50);
/// How many bytes of the messages read before are kept, to show again at
/// once: the latest read, and as many before it as fit.
const KEPT: usize = 64 << 20;

#[derive(Clone, Copy, PartialEq, Debug)]
pub enum Focus {
    Side,
    List,
    Reader,
}

pub enum State {
    Connecting,
    Ready,
    Failed(String),
}

pub struct Account {
    pub name: String,
    /// Its address, once the store has said it: the ledger's name for it,
    /// and what a message is sent from.
    pub email: String,
    pub folders: Vec<Folder>,
    /// The folder `d` moves mail to, when the server marks none as Trash.
    /// Taken from the account when it joins, and kept in the accounts file.
    pub trash: Option<String>,
    pub state: State,
    /// To ask its store; none once it is removed, and its thread ends.
    asks: Option<Sender<Ask>>,
}

/// What the sidebar lists: with more than one account, their Inbox, Sent
/// and Drafts together, under a heading; each account, and its folders
/// under it; and a row to add another.
#[derive(Clone, Copy, PartialEq, Debug)]
pub enum Row {
    Everyone,
    All(Role),
    Account(usize),
    Folder(usize, usize),
    Add,
}

/// The fields of the form an account is added with, in their order; where
/// its password is kept is picked rather than typed, and the vault's
/// passphrase asked for only if it is kept there and the vault is locked,
/// twice if there is no vault yet.
pub const FIELDS: [&str; 10] = [
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
];
pub(crate) const EMAIL: usize = 0;
pub(crate) const NAME: usize = 1;
pub(crate) const SERVER: usize = 2;
const PORT: usize = 3;
const USER: usize = 4;
pub const SIGN_IN: usize = 5;
pub const PASSWORD: usize = 6;
pub const KEEP: usize = 7;
pub const PASSPHRASE: usize = 8;
pub const AGAIN: usize = 9;

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
    pub fields: [String; 10],
    pub at: usize,
    typed: [bool; 10],
    /// How it signs in: with a password, or on Microsoft's page.
    pub auth: Auth,
    /// The address Microsoft was last asked about.
    asked: Option<String>,
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
    stop: Arc<AtomicBool>,
}

impl Form {
    fn new(keepers: Vec<Keeper>) -> Self {
        let mut fields: [String; 10] = Default::default();
        fields[PORT] = "993".to_string();
        Form {
            fields,
            at: EMAIL,
            typed: [false; 10],
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
        let mut all: Vec<usize> = (EMAIL..=SIGN_IN).collect();
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
    fn last(&self) -> bool {
        self.at >= PASSWORD || (self.at == SIGN_IN && self.auth == Auth::Microsoft)
    }

    /// Go to the field `by` after the one typed in, among those shown.
    fn step(&mut self, by: isize) {
        let shown = self.shown();
        let at = shown.iter().position(|&i| i == self.at).unwrap_or(0) as isize;
        let n = shown.len() as isize;
        self.at = shown[(at + by).rem_euclid(n) as usize];
    }

    /// Sign in the other way.
    fn flip(&mut self) {
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
    fn microsoft(&mut self) {
        if self.typed[SIGN_IN] {
            return;
        }
        self.auth = Auth::Microsoft;
        if !self.typed[SERVER] {
            self.fields[SERVER] = oauth::IMAP_HOST.to_string();
        }
        if self.failed.is_none() {
            self.note = Some(config::MICROSOFT_NOTE.to_string());
        }
    }

    /// Keep the password in the next place it may be kept, or the one before.
    fn turn(&mut self, by: isize) {
        let n = self.keepers.len().max(1) as isize;
        self.keeper = (self.keeper as isize + by).rem_euclid(n) as usize;
    }

    /// Fill in what the address tells, where nothing was typed.
    fn fill(&mut self) {
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

/// The roles whose folders are listed together, every account's, in the
/// sidebar's order.
const TOGETHER: [Role; 3] = [Role::Inbox, Role::Sent, Role::Drafts];

/// What the list shows: a folder of one account's, or every account's
/// folder of a role.
#[derive(Clone, PartialEq, Debug)]
pub enum View {
    Folder(usize, String),
    All(Role),
}

/// A folder's messages, as many as are listed, newest first as its server
/// numbers them; and how many it holds.
pub(crate) struct Part {
    pub account: usize,
    pub folder: String,
    pub messages: Vec<Summary>,
    pub total: u32,
    pub loading: bool,
    pub error: Option<String>,
}

impl Part {
    fn is(&self, account: usize, folder: &str) -> bool {
        self.account == account && self.folder == folder
    }

    /// Whether it holds more than is listed.
    fn partial(&self) -> bool {
        self.messages.len() < self.total as usize
    }

    /// The date of the oldest dated message listed.
    fn oldest(&self) -> Option<i64> {
        self.messages.iter().filter_map(|m| m.date).min()
    }
}

/// How old a message of folders together may be and still be listed: as
/// old as the newest of the oldest listed of the folders listed in part,
/// as what is still to come of them would come before anything older.
fn cut(parts: &[Part]) -> Option<i64> {
    parts
        .iter()
        .filter(|p| p.partial())
        .filter_map(Part::oldest)
        .max()
}

/// What is listed: the folders the view shows, each as far as it is.
pub struct Listing {
    pub view: View,
    pub(crate) parts: Vec<Part>,
}

impl Listing {
    fn part_mut(&mut self, account: usize, folder: &str) -> Option<&mut Part> {
        self.parts.iter_mut().find(|p| p.is(account, folder))
    }

    fn has(&self, account: usize, folder: &str) -> bool {
        self.parts.iter().any(|p| p.is(account, folder))
    }

    /// Each folder listed, by its account and path.
    fn keys(&self) -> Vec<(usize, String)> {
        self.parts
            .iter()
            .map(|p| (p.account, p.folder.clone()))
            .collect()
    }

    /// The account whose folder is listed, if it is one account's.
    pub fn account(&self) -> Option<usize> {
        match self.view {
            View::Folder(a, _) => Some(a),
            View::All(_) => None,
        }
    }

    /// Whether more than one account's messages are listed, each to be
    /// told by its account.
    pub fn mixed(&self) -> bool {
        self.parts
            .iter()
            .any(|p| p.account != self.parts[0].account)
    }

    pub fn loading(&self) -> bool {
        self.parts.iter().any(|p| p.loading)
    }

    /// Whether messages are still to come before any is listed.
    pub fn fetching(&self) -> bool {
        self.loading() && self.parts.iter().all(|p| p.messages.is_empty())
    }

    pub fn total(&self) -> u32 {
        self.parts.iter().map(|p| p.total).sum()
    }

    pub fn error(&self) -> Option<&str> {
        self.parts.iter().find_map(|p| p.error.as_deref())
    }
}

/// A message listed, and the account and folder it is in.
#[derive(Clone, Debug)]
pub struct Listed {
    pub account: usize,
    pub folder: String,
    pub summary: Summary,
}

impl std::ops::Deref for Listed {
    type Target = Summary;
    fn deref(&self) -> &Summary {
        &self.summary
    }
}

impl Listed {
    fn new(account: usize, folder: &str, summary: Summary) -> Self {
        Listed {
            account,
            folder: folder.to_string(),
            summary,
        }
    }

    fn is(&self, account: usize, folder: &str, uid: u32) -> bool {
        self.account == account && self.folder == folder && self.uid == uid
    }
}

/// The messages of `parts`, as listed, with the folder each is in: one
/// folder's as its server numbers them; folders together newest first,
/// the undated last, and none older than the cut.
pub(crate) fn merged(parts: &[Part]) -> Vec<(&Part, &Summary)> {
    let mut all: Vec<(&Part, &Summary)> = parts
        .iter()
        .flat_map(|p| p.messages.iter().map(move |m| (p, m)))
        .collect();
    if let [_] = parts {
        return all;
    }
    let cut = cut(parts);
    all.retain(|(_, m)| cut.is_none_or(|c| m.date.is_none_or(|d| d >= c)));
    all.sort_by_key(|(_, m)| newest(m));
    all
}

/// What sorts messages of folders together newest first; the undated last.
fn newest(m: &Summary) -> std::cmp::Reverse<i64> {
    std::cmp::Reverse(m.date.unwrap_or(i64::MIN))
}

pub enum Body {
    Loading,
    Ready(Arc<Message>),
    Failed(String),
}

/// The message read.
pub struct Reading {
    pub account: usize,
    pub folder: String,
    pub uid: u32,
    pub body: Body,
}

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
    /// An attachment or link in the list of them, by its place.
    Item(usize),
}

#[derive(Clone, Copy, PartialEq, Debug)]
pub enum Button {
    Add,
    Cancel,
    Remove,
    Keep,
}

/// What the message read has to open: an attachment, by its place among
/// its attachments, or a link.
#[derive(Clone, Debug, PartialEq)]
pub enum Item {
    File {
        at: usize,
        name: String,
        mime: String,
        size: usize,
    },
    Link(String),
}

/// A message being written, in the pane that covers the list and the
/// message read.
pub struct Compose {
    pub buffer: edit::Buffer,
    pub account: usize,
    /// What it answers, if anything.
    thread: Thread,
    /// `C-c` was pressed, and the next key sends or cancels.
    pub prefix: bool,
    pub scroll: usize,
    /// A send has been asked and not yet answered; the pane stays up.
    pub sending: bool,
    /// The files it is sent with: a forward's, the message's it forwards.
    pub files: Vec<compose::File>,
}

/// A message to answer once it is read: where it is, and how.
struct Pending {
    account: usize,
    folder: String,
    uid: u32,
    how: Respond,
}

/// The existing folders letterbox is asking which of is Trash, the one
/// highlighted, for an account whose server marks none.
pub struct TrashPick {
    pub account: usize,
    pub folders: Vec<(String, String)>,
    pub at: usize,
}

/// A delete waiting until the folder that is Trash is known.
struct Held {
    account: usize,
    from: String,
    uid: u32,
    message_id: String,
}

/// The message's attachments and links, listed to open, the one picked,
/// and what came of the last opened.
pub struct Files {
    pub items: Vec<Item>,
    pub at: usize,
    pub note: Option<String>,
}

/// What is asked over the screen, one thing at a time: the keys' help,
/// scrolled so far; the form to add an account; the message's files and
/// links; which folder is Trash; whether to remove an account; what to
/// call one, as typed so far.
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

    fn form_mut(&mut self) -> Option<&mut Form> {
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

    fn files_mut(&mut self) -> Option<&mut Files> {
        match self {
            Dialog::Files(files) => Some(files),
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
    fn step(&mut self, by: isize) {
        let (at, len) = match self {
            Dialog::Help(scroll) => {
                *scroll = scroll.saturating_add_signed(by);
                return;
            }
            Dialog::Form(form) if !form.checking => return form.step(by),
            Dialog::Files(files) => (&mut files.at, files.items.len()),
            Dialog::Trash(pick) => (&mut pick.at, pick.folders.len()),
            _ => return,
        };
        *at = at.saturating_add_signed(by).min(len.saturating_sub(1));
    }
}

/// What the terminal is given over to, or the system asked to do, once
/// the screen is drawn: a document presented by ohp, which takes the
/// terminal until it quits; a file opened by the system; a page in the
/// browser; an address put on the clipboard, by the terminal.
#[derive(Clone, Debug, PartialEq)]
pub enum Launch {
    Ohp(PathBuf),
    Open(PathBuf),
    Browse(String),
    Copy(String),
}

/// Whether ohp presents a file of this name: PDFs, and markdown and R
/// Markdown, which it typesets.
pub fn ohp_reads(name: &str) -> bool {
    let name = name.to_lowercase();
    [".pdf", ".md", ".markdown", ".rmd"]
        .iter()
        .any(|e| name.ends_with(e))
}

/// What waits for a pause in moving through a list.
#[derive(Clone, Copy, PartialEq)]
enum Due {
    Folder,
    Message,
}

/// A message, by its account, folder and UID.
type Key = (usize, String, u32);

/// The messages read before, the oldest let go once they hold more than
/// `KEPT` bytes.
#[derive(Default)]
struct Kept {
    messages: HashMap<Key, Arc<Message>>,
    /// The order they were read in, the oldest first.
    order: VecDeque<Key>,
    bytes: usize,
}

impl Kept {
    fn get(&self, key: &Key) -> Option<Arc<Message>> {
        self.messages.get(key).cloned()
    }

    fn insert(&mut self, key: Key, message: Arc<Message>) {
        self.bytes += bytes(&message);
        if let Some(old) = self.messages.insert(key.clone(), message) {
            self.bytes -= bytes(&old);
            self.order.retain(|k| *k != key);
        }
        self.order.push_back(key);
        while self.bytes > KEPT && self.order.len() > 1 {
            let Some(oldest) = self.order.pop_front() else {
                break;
            };
            if let Some(m) = self.messages.remove(&oldest) {
                self.bytes -= bytes(&m);
            }
        }
    }

    /// Let go of the account's messages.
    fn forget(&mut self, account: usize) {
        self.messages.retain(|(a, _, _), _| *a != account);
        self.order.retain(|(a, _, _)| *a != account);
        self.bytes = self.messages.values().map(|m| bytes(m)).sum();
    }
}

/// About how much of memory a message read takes.
fn bytes(m: &Message) -> usize {
    m.raw.len() + m.text.len() + m.html.as_ref().map_or(0, String::len)
}

pub struct App {
    pub accounts: Vec<Account>,
    pub rows: Vec<Row>,
    pub row: usize,
    pub listing: Option<Listing>,
    /// Folders shown before, as they were, to show again at once.
    listed: HashMap<(usize, String), (Vec<Summary>, u32)>,
    /// What is listed: the folder's messages, or those the search finds.
    pub shown: Vec<Listed>,
    pub list: ListState,
    pub query: String,
    pub searching: bool,
    /// What the server found for the query, once asked with ⏎.
    pub found: Option<Vec<Listed>>,
    pub reading: Option<Reading>,
    /// How far the message read is scrolled, and how many lines it has and
    /// shows, as last drawn.
    pub scroll: usize,
    pub lines: usize,
    pub height: usize,
    /// Whether the message read is on the screen beside the list.
    pub side_by_side: bool,
    /// The message read as last drawn, its width and colours, and its
    /// lines: wrapped once, not again each time the screen is drawn.
    pub drawn: Option<(Arc<Message>, usize, Theme, Vec<Line<'static>>)>,
    read: Kept,
    pub focus: Focus,
    pub theme: Theme,
    pub status: String,
    pub targets: Vec<(Rect, Target)>,
    /// Whether to mark a message read when it is shown.
    pub mark_read: bool,
    /// Where the accounts are set up, to say so where there are none.
    pub config: PathBuf,
    /// Whether accounts may be added and removed: not the demo's.
    pub editable: bool,
    /// Where an account's password may be kept, and how, once it signs in.
    pub keepers: fn() -> Vec<Keeper>,
    pub keep: setup::Keep,
    /// How a page is opened in the browser, as Microsoft's to sign in on.
    pub browse: fn(&str),
    /// What is asked over the screen, if anything.
    pub dialog: Dialog,
    /// A message being written, over the list and the message read.
    pub compose: Option<Compose>,
    pub(crate) ledger: trash::Ledger,
    held: Option<Held>,
    /// A reply waiting on the message it answers, fetched first.
    reply_when: Option<Pending>,
    /// Trash removals asked and not yet answered, by address, folder and UID,
    /// so one is not asked twice.
    purging: HashSet<(String, String, u32)>,
    /// What to do outside the screen, once it is drawn.
    pub launch: Option<Launch>,
    replies: Receiver<Reply>,
    replies_to: Sender<Reply>,
    /// Asks not yet answered.
    pending: usize,
    due: Option<(Instant, Due)>,
    refreshed: Instant,
    clicked: Option<(Instant, usize)>,
    quit: bool,
}

pub fn run(app: &mut App) -> anyhow::Result<()> {
    let mut terminal = take();
    let result = (|| -> anyhow::Result<()> {
        let mut stale = true;
        while !app.quit {
            if stale {
                terminal.draw(|f| ui::draw(f, app))?;
            }
            if let Some(launch) = app.launch.take() {
                app.status = carry_out(launch, &mut terminal);
                terminal.clear()?;
                stale = true;
                continue;
            }
            stale = app.pump() | app.tick();
            if !event::poll(POLL)? {
                continue;
            }
            let event = event::read()?;
            stale |= !matches!(
                event,
                Event::Mouse(MouseEvent {
                    kind: MouseEventKind::Moved | MouseEventKind::Drag(_),
                    ..
                }) | Event::FocusGained
                    | Event::FocusLost
            );
            match event {
                Event::Key(key) if key.kind == KeyEventKind::Press => app.key(key),
                Event::Mouse(m) => app.mouse(m),
                _ => {}
            }
        }
        Ok(())
    })();
    give_back();
    result
}

/// Do what `launch` asks, outside the screen; what to say of it.
fn carry_out(launch: Launch, terminal: &mut DefaultTerminal) -> String {
    match launch {
        Launch::Ohp(path) => {
            give_back();
            let ran = std::process::Command::new("ohp").arg(&path).status();
            *terminal = take();
            match ran {
                Ok(s) if s.success() => String::new(),
                Ok(s) => format!("ohp: {s}"),
                Err(_) => match open(&path) {
                    Ok(()) => {
                        "ohp is not installed (cargo install ohp): opened by the system".to_string()
                    }
                    Err(e) => format!("cannot open {}: {e:#}", path.display()),
                },
            }
        }
        Launch::Open(path) => match open(&path) {
            Ok(()) => format!("opened {}", tilde(&path)),
            Err(e) => format!("cannot open {}: {e:#}", path.display()),
        },
        Launch::Browse(url) => {
            oauth::browse(&url);
            "opened in the browser".to_string()
        }
        Launch::Copy(text) => {
            // OSC 52: the terminal puts it on the clipboard, here or over ssh.
            use base64::Engine;
            let code = base64::engine::general_purpose::STANDARD.encode(&text);
            let _ = std::io::Write::write_all(
                &mut std::io::stdout(),
                format!("\x1b]52;c;{code}\x07").as_bytes(),
            );
            "copied the address".to_string()
        }
    }
}

/// Whether the terminal tells apart keys that it otherwise sends alike, as
/// Ctrl+Shift+R and Ctrl+R: asked once, as it answers each time it is asked.
static TELLS_APART: std::sync::OnceLock<bool> = std::sync::OnceLock::new();

/// The terminal, as ratatui takes it, with the mouse's clicks and wheel,
/// and keys told apart where the terminal can.
fn take() -> DefaultTerminal {
    let terminal = ratatui::init();
    let _ = execute!(std::io::stdout(), EnableMouseCapture);
    if *TELLS_APART.get_or_init(|| {
        ratatui::crossterm::terminal::supports_keyboard_enhancement().unwrap_or(false)
    }) {
        let _ = execute!(
            std::io::stdout(),
            PushKeyboardEnhancementFlags(KeyboardEnhancementFlags::DISAMBIGUATE_ESCAPE_CODES)
        );
    }
    terminal
}

fn give_back() {
    if TELLS_APART.get() == Some(&true) {
        let _ = execute!(std::io::stdout(), PopKeyboardEnhancementFlags);
    }
    let _ = execute!(std::io::stdout(), DisableMouseCapture);
    ratatui::restore();
}

impl App {
    /// The app over `stores`, each an account by its name; their folders
    /// are asked for at once.
    pub fn new(stores: Vec<(String, Box<dyn Store>)>, config: PathBuf) -> Self {
        let (replies_to, replies) = mpsc::channel();
        let ledger = trash::Ledger::load();
        let status = ledger.fault().unwrap_or("").to_string();
        let mut app = App {
            accounts: Vec::new(),
            rows: Vec::new(),
            row: 0,
            listing: None,
            listed: HashMap::new(),
            shown: Vec::new(),
            list: ListState::default(),
            query: String::new(),
            searching: false,
            found: None,
            reading: None,
            scroll: 0,
            lines: 0,
            height: 0,
            side_by_side: true,
            drawn: None,
            read: Kept::default(),
            focus: Focus::List,
            theme: Theme::of(theme::Name::default()),
            status,
            targets: Vec::new(),
            mark_read: true,
            config,
            editable: true,
            keepers: crate::secret::keepers,
            keep: crate::secret::store,
            browse: oauth::browse,
            dialog: Dialog::Closed,
            compose: None,
            ledger,
            held: None,
            reply_when: None,
            purging: HashSet::new(),
            launch: None,
            replies,
            replies_to,
            pending: 0,
            due: None,
            refreshed: Instant::now(),
            clicked: None,
            quit: false,
        };
        for (name, store) in stores {
            let a = app.join(name, store);
            app.ask(a, Ask::Folders);
        }
        app.rebuild();
        app
    }

    /// Keep the accounts as they are, none added or removed: the demo's.
    pub fn fix_accounts(&mut self) {
        self.editable = false;
        self.rebuild();
    }

    /// Take on an account, its store asked from a thread of its own; its
    /// place among the accounts.
    fn join(&mut self, name: String, store: Box<dyn Store>) -> usize {
        let a = self.accounts.len();
        let trash = store.trash().map(str::to_string);
        let asks = worker::spawn(a, store, self.replies_to.clone());
        self.accounts.push(Account {
            name,
            email: String::new(),
            folders: Vec::new(),
            trash,
            state: State::Connecting,
            asks: Some(asks),
        });
        a
    }

    fn ask(&mut self, account: usize, ask: Ask) {
        let sent = self.accounts[account]
            .asks
            .as_ref()
            .is_some_and(|s| s.send(ask).is_ok());
        if sent {
            self.pending += 1;
        }
    }

    /// The accounts not removed, by their place.
    pub fn live(&self) -> impl Iterator<Item = usize> + '_ {
        (0..self.accounts.len()).filter(|&a| self.accounts[a].asks.is_some())
    }

    /// Take what the stores have answered; whether anything was.
    pub fn pump(&mut self) -> bool {
        let mut any = false;
        while let Ok(reply) = self.replies.try_recv() {
            self.take_reply(reply);
            any = true;
        }
        any
    }

    /// Wait until the stores have answered all that was asked, and what
    /// waits for a pause is done, or until `limit`: for tests and
    /// `letterbox screen`.
    pub fn settle(&mut self, limit: Duration) {
        let until = Instant::now() + limit;
        loop {
            if let Some((_, due)) = self.due.take() {
                self.run_due(due);
            }
            if self.pending == 0 {
                return;
            }
            let left = until.saturating_duration_since(Instant::now());
            match self.replies.recv_timeout(left) {
                Ok(reply) => self.take_reply(reply),
                Err(_) => return,
            }
        }
    }

    /// Do what has waited long enough; whether anything was done.
    pub fn tick(&mut self) -> bool {
        let mut any = false;
        if let Some((at, due)) = self.due
            && at.elapsed() >= PAUSE
        {
            self.due = None;
            self.run_due(due);
            any = true;
        }
        if self.refreshed.elapsed() >= REFRESH {
            self.refresh();
            any = true;
        }
        any
    }

    fn run_due(&mut self, due: Due) {
        match due {
            Due::Folder => self.open_folder(),
            Due::Message => self.read_picked(),
        }
    }

    fn take_reply(&mut self, reply: Reply) {
        let a = reply.account;
        // How to sign in comes while signing in, not in answer to an ask.
        if let Answer::Waiting(waiting) = reply.answer {
            if let Some(form) = self.dialog.form_mut() {
                form.note = None;
                form.waiting = Some(waiting);
            }
            return;
        }
        // The address is said as the thread starts, not in answer to an ask.
        if let Answer::Address(email) = reply.answer {
            if let Some(account) = self.accounts.get_mut(a) {
                account.email = email;
            }
            self.sweep(a);
            return;
        }
        self.pending = self.pending.saturating_sub(1);
        if let Answer::Added(added) = reply.answer {
            return self.added(added);
        }
        if let Answer::Fetched(got) = reply.answer {
            match got {
                Ok(path) => {
                    if let Some(files) = self.dialog.files_mut() {
                        files.note = None;
                    }
                    self.launch = Some(Launch::Ohp(path));
                }
                Err(e) => match self.dialog.files_mut() {
                    Some(files) => files.note = Some(e),
                    None => self.status = first_line(&e).to_string(),
                },
            }
            return;
        }
        if let Answer::Realm { email, microsoft } = reply.answer {
            if let Some(form) = self.dialog.form_mut()
                && microsoft
                && form.fields[EMAIL].trim() == email
                && !form.checking
            {
                form.microsoft();
            }
            return;
        }
        if self.accounts.get(a).is_none_or(|x| x.asks.is_none()) {
            return;
        }
        match reply.answer {
            Answer::Folders(Ok(folders)) => {
                let first = self.accounts.iter().all(|x| x.folders.is_empty());
                self.accounts[a].folders = folders;
                self.remember_trash(a);
                self.accounts[a].state = State::Ready;
                self.rebuild();
                if first && self.listing.is_none() {
                    let inbox = self.accounts[a]
                        .folders
                        .iter()
                        .position(|f| f.selectable)
                        .and_then(|i| self.rows.iter().position(|r| *r == Row::Folder(a, i)));
                    if let Some(row) = inbox {
                        self.row = row;
                        self.open_folder();
                    }
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
                self.join_view(a);
                self.sweep(a);
                self.resume_held(a);
            }
            Answer::Folders(Err(e)) => {
                self.status = format!("{}: {}", self.accounts[a].name, first_line(&e));
                self.accounts[a].state = State::Failed(e);
                self.rebuild();
            }
            Answer::Count {
                folder,
                counts: Ok((total, unseen)),
            } => {
                if let Some(f) = self.folder_mut(a, &folder) {
                    f.total = Some(total);
                    f.unseen = Some(unseen);
                }
            }
            Answer::Count { counts: Err(_), .. }
            | Answer::Skipped
            | Answer::Added(_)
            | Answer::Waiting(_)
            | Answer::Address(_)
            | Answer::Fetched(_)
            | Answer::Realm { .. } => {}
            Answer::Page {
                folder,
                offset,
                page,
            } => {
                let Some(l) = self.listing.as_mut().and_then(|l| l.part_mut(a, &folder)) else {
                    return;
                };
                l.loading = false;
                match page {
                    Ok(page) => {
                        l.error = None;
                        l.total = page.total;
                        if offset == 0 {
                            // The newest, and those listed before older than them.
                            let oldest = page.summaries.iter().map(|s| s.uid).min();
                            let full = page.summaries.len() >= PAGE;
                            let mut messages = page.summaries;
                            if full && let Some(oldest) = oldest {
                                messages.extend(l.messages.drain(..).filter(|m| m.uid < oldest));
                            }
                            l.messages = messages;
                        } else {
                            let have: std::collections::HashSet<u32> =
                                l.messages.iter().map(|m| m.uid).collect();
                            l.messages.extend(
                                page.summaries
                                    .into_iter()
                                    .filter(|s| !have.contains(&s.uid)),
                            );
                        }
                        let total = l.total;
                        if let Some(f) = self.folder_mut(a, &folder) {
                            f.total = Some(total);
                        }
                    }
                    Err(e) => {
                        self.status = first_line(&e).to_string();
                        l.error = Some(e);
                    }
                }
                self.narrow();
            }
            Answer::Opened {
                folder,
                uid,
                message,
                marked,
            } => {
                if let Ok(m) = &message {
                    self.read.insert((a, folder.clone(), uid), m.clone());
                }
                if marked {
                    self.marked(a, &folder, uid, Flag::Seen, true);
                }
                if let Some(r) = self
                    .reading
                    .as_mut()
                    .filter(|r| r.account == a && r.folder == folder && r.uid == uid)
                {
                    r.body = match message {
                        Ok(m) => Body::Ready(m),
                        Err(e) => Body::Failed(e),
                    };
                }
                if let Some(p) = self
                    .reply_when
                    .take_if(|p| p.account == a && p.folder == folder && p.uid == uid)
                    && let Some(m) = self.read.get(&(a, folder.clone(), uid))
                {
                    self.open_reply(a, &folder, &m, p.how);
                }
            }
            Answer::Set {
                folder,
                uid,
                flag,
                on,
                result,
            } => {
                if let Err(e) = result {
                    self.status = format!("cannot mark it: {}", first_line(&e));
                    self.marked(a, &folder, uid, flag, !on);
                }
            }
            Answer::Found {
                folder,
                words,
                found,
            } => {
                let current = self.listing.as_ref().is_some_and(|l| l.has(a, &folder));
                if !current || words != self.query.trim() {
                    return;
                }
                match found {
                    Ok(found) => {
                        // Each account's answer joins those come before.
                        let mut all = self.found.take().unwrap_or_default();
                        all.retain(|m| m.account != a || m.folder != folder);
                        all.extend(found.into_iter().map(|s| Listed::new(a, &folder, s)));
                        // In the list's order: folders together newest first.
                        if self.listing.as_ref().is_some_and(Listing::mixed) {
                            all.sort_by_key(|m| newest(m));
                        }
                        self.status = format!("{} found in {}", all.len(), self.folder_name());
                        self.found = Some(all);
                    }
                    Err(e) => self.status = format!("cannot search: {}", first_line(&e)),
                }
                self.narrow();
            }
            Answer::Moved {
                from,
                uid,
                to,
                message_id,
                trashed,
                new_uid,
                email,
            } => match new_uid {
                Ok(new_uid) => {
                    if !email.is_empty() {
                        self.accounts[a].email = email.clone();
                    }
                    if trashed {
                        let at = jiff::Timestamp::now().as_second();
                        if let Err(e) = self.ledger.record(Entry {
                            account: email,
                            message_id,
                            folder: to.clone(),
                            uid: new_uid,
                            origin: from.clone(),
                            at,
                        }) {
                            self.status = format!("moved to Trash, and not recorded: {e:#}");
                        }
                    } else {
                        let email = self.accounts[a].email.clone();
                        self.forget_trashed(&email, &from, uid);
                    }
                    self.ask(a, Ask::Count(from));
                    self.ask(a, Ask::Count(to));
                    self.reload(a);
                }
                Err(e) => {
                    self.status = format!("cannot move it: {}", first_line(&e));
                    self.reload(a);
                }
            },
            Answer::Expunged {
                folder,
                uid,
                result,
            } => {
                let email = self.accounts[a].email.clone();
                self.purging.remove(&(email.clone(), folder.clone(), uid));
                match result {
                    Ok(Removal::Gone) => {
                        self.forget_trashed(&email, &folder, uid);
                        self.drop_listed(a, &folder, uid);
                        self.ask(a, Ask::Count(folder));
                    }
                    Ok(Removal::Missing) => self.forget_trashed(&email, &folder, uid),
                    Err(e) => self.status = format!("cannot remove it: {}", first_line(&e)),
                }
            }
            Answer::Sent { said, answer } => {
                if let Some(compose) = &mut self.compose {
                    compose.sending = false;
                }
                match said {
                    Ok(said) => {
                        if let Some((folder, uid)) = answer {
                            self.marked(a, &folder, uid, Flag::Answered, true);
                        }
                        self.compose = None;
                        self.status = said;
                        self.reload(a);
                    }
                    Err(e) => self.status = first_line(&e).to_string(),
                }
            }
        }
    }

    fn folder_mut(&mut self, account: usize, path: &str) -> Option<&mut Folder> {
        self.accounts[account]
            .folders
            .iter_mut()
            .find(|f| f.path == path)
    }

    /// The name of what is listed: a folder's, or a role's, of every
    /// account's.
    pub fn folder_name(&self) -> String {
        match self.listing.as_ref().map(|l| &l.view) {
            Some(View::All(role)) => format!("{}, all accounts", role.name()),
            Some(View::Folder(..)) => self.folder().map(|f| f.name.clone()).unwrap_or_default(),
            None => String::new(),
        }
    }

    /// The folder listed, as the sidebar has it, if one account's is.
    pub fn folder(&self) -> Option<&Folder> {
        match &self.listing.as_ref()?.view {
            View::Folder(a, path) => self.folder_of(*a, path),
            View::All(_) => None,
        }
    }

    fn folder_of(&self, account: usize, path: &str) -> Option<&Folder> {
        self.accounts
            .get(account)?
            .folders
            .iter()
            .find(|f| f.path == path)
    }

    fn is_trash(&self, account: usize, path: &str) -> bool {
        self.folder_of(account, path)
            .is_some_and(|f| f.role == Role::Trash)
    }

    /// The account's folder of `role`, if it has one to list.
    fn role_folder(&self, account: usize, role: Role) -> Option<&Folder> {
        self.accounts[account]
            .folders
            .iter()
            .find(|f| f.selectable && f.role == role)
    }

    /// The folders of `role`, one an account, of the accounts in use.
    fn folders_of(&self, role: Role) -> Vec<(usize, String)> {
        self.live()
            .filter_map(|a| Some((a, self.role_folder(a, role)?.path.clone())))
            .collect()
    }

    /// How many messages of every account's folder of `role` are not read,
    /// as far as their folders are counted.
    pub fn unseen_of(&self, role: Role) -> Option<u32> {
        self.live()
            .filter_map(|a| self.role_folder(a, role)?.unseen)
            .reduce(|x, y| x + y)
    }

    /// Show a message as marked, or not, wherever it is listed, and count
    /// its folder's unread messages again.
    fn marked(&mut self, account: usize, folder: &str, uid: u32, flag: Flag, on: bool) {
        let mut changed = false;
        let mut set = |s: &mut Summary| {
            if s.uid != uid {
                return;
            }
            let field = match flag {
                Flag::Seen => &mut s.flags.seen,
                Flag::Flagged => &mut s.flags.flagged,
                Flag::Answered => &mut s.flags.answered,
            };
            changed |= *field != on;
            *field = on;
        };
        if let Some(l) = self
            .listing
            .as_mut()
            .and_then(|l| l.part_mut(account, folder))
        {
            l.messages.iter_mut().for_each(&mut set);
            std::iter::once(&mut self.shown)
                .chain(self.found.as_mut())
                .flatten()
                .filter(|m| m.account == account && m.folder == folder)
                .for_each(|m| set(&mut m.summary));
        }
        if let Some((messages, _)) = self.listed.get_mut(&(account, folder.to_string())) {
            messages.iter_mut().for_each(&mut set);
        }
        if changed
            && flag == Flag::Seen
            && let Some(f) = self.folder_mut(account, folder)
            && let Some(n) = &mut f.unseen
        {
            *n = if on {
                n.saturating_sub(1)
            } else {
                n.saturating_add(1)
            };
        }
    }

    /// The sidebar's rows again, the one picked kept.
    fn rebuild(&mut self) {
        let picked = self.rows.get(self.row).copied();
        self.rows.clear();
        if self.live().count() > 1 {
            self.rows.push(Row::Everyone);
            self.rows.extend(TOGETHER.map(Row::All));
        }
        for a in self.live().collect::<Vec<_>>() {
            self.rows.push(Row::Account(a));
            self.rows
                .extend((0..self.accounts[a].folders.len()).map(|i| Row::Folder(a, i)));
        }
        if self.editable {
            self.rows.push(Row::Add);
        }
        self.row = picked
            .and_then(|p| self.rows.iter().position(|r| *r == p))
            .or_else(|| self.rows.iter().position(|r| matches!(r, Row::Folder(..))))
            .unwrap_or(0);
    }

    /// Open the form to add an account.
    fn open_form(&mut self) {
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
    fn submit(&mut self) {
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
    fn ask_microsoft(&mut self) {
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
    fn added(&mut self, done: Result<Added, String>) {
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
    fn remove(&mut self, a: usize) {
        let name = self.accounts[a].name.clone();
        let forgotten = match setup::remove(&self.config, &name, false) {
            Ok((_, forgotten)) => forgotten,
            Err(e) => {
                self.status = format!("cannot remove {name}: {e:#}");
                return;
            }
        };
        self.accounts[a].asks = None;
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
    fn form_key(&mut self, key: KeyEvent) {
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
        let Some(form) = self.dialog.form_mut() else {
            return;
        };
        if form.at == EMAIL && !form.checking {
            form.failed = None;
            form.fill();
        }
        if before == EMAIL && form.at != EMAIL {
            self.ask_microsoft();
        }
    }

    pub fn current(&self) -> Option<&Listed> {
        self.shown.get(self.list.selected()?)
    }

    /// List what is picked in the sidebar, as it was if it was listed
    /// before, and ask for its newest messages.
    fn open_folder(&mut self) {
        let view = match self.rows.get(self.row) {
            Some(&Row::Folder(a, i)) => {
                let Some(folder) = self.accounts[a].folders.get(i) else {
                    return;
                };
                if !folder.selectable {
                    return;
                }
                View::Folder(a, folder.path.clone())
            }
            Some(&Row::All(role)) => View::All(role),
            _ => return,
        };
        if self.listing.as_ref().is_some_and(|l| l.view == view) {
            return;
        }
        if let Some(old) = self.listing.take() {
            for p in old.parts {
                self.listed
                    .insert((p.account, p.folder), (p.messages, p.total));
            }
        }
        self.listing = Some(Listing {
            view: view.clone(),
            parts: Vec::new(),
        });
        self.query.clear();
        self.found = None;
        self.searching = false;
        // What was picked before is not picked again: the list starts at
        // its top.
        self.shown.clear();
        self.list.select(Some(0));
        *self.list.offset_mut() = 0;
        let folders = match view {
            View::Folder(a, path) => vec![(a, path)],
            View::All(role) => self.folders_of(role),
        };
        for (a, path) in folders {
            self.list_part(a, path);
        }
        self.narrow();
    }

    /// List one account's folder in what is listed, as it was if it was
    /// listed before, and ask for its newest messages.
    fn list_part(&mut self, a: usize, path: String) {
        let Some(listing) = &mut self.listing else {
            return;
        };
        let (messages, total) = self.listed.remove(&(a, path.clone())).unwrap_or_default();
        listing.parts.push(Part {
            account: a,
            folder: path.clone(),
            messages,
            total,
            loading: true,
            error: None,
        });
        let trash = self.is_trash(a, &path);
        self.ask_page(a, path, 0);
        if trash {
            self.sweep(a);
        }
    }

    /// Join the account's folder to the folders listed together, once its
    /// folders are known.
    fn join_view(&mut self, a: usize) {
        let Some(View::All(role)) = self.listing.as_ref().map(|l| l.view.clone()) else {
            return;
        };
        let Some(path) = self.role_folder(a, role).map(|f| f.path.clone()) else {
            return;
        };
        if !self.listing.as_ref().is_some_and(|l| l.has(a, &path)) {
            self.list_part(a, path.clone());
            // What is searched for is searched in its folder too.
            if self.found.is_some() {
                self.ask_search(a, path);
            }
            self.narrow();
        }
    }

    /// Ask for a page of the folder's messages, `offset` from its newest.
    fn ask_page(&mut self, account: usize, folder: String, offset: usize) {
        self.ask(
            account,
            Ask::Page {
                folder,
                offset,
                n: PAGE,
            },
        );
    }

    /// Ask the folder's server for what the query finds in it.
    fn ask_search(&mut self, account: usize, folder: String) {
        let words = self.query.trim().to_string();
        if !words.is_empty() {
            self.ask(
                account,
                Ask::Search {
                    folder,
                    words,
                    n: FOUND,
                },
            );
        }
    }

    /// Look in the folder listed again, and count every folder's messages.
    pub fn refresh(&mut self) {
        self.refreshed = Instant::now();
        let parts: Vec<(usize, String)> = self
            .listing
            .iter_mut()
            .flat_map(|l| &mut l.parts)
            .map(|p| {
                p.loading = true;
                (p.account, p.folder.clone())
            })
            .collect();
        for (a, folder) in parts {
            self.ask_page(a, folder, 0);
        }
        for a in self.live().collect::<Vec<_>>() {
            if matches!(self.accounts[a].state, State::Failed(_)) {
                self.accounts[a].state = State::Connecting;
                self.ask(a, Ask::Folders);
                continue;
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
        }
    }

    /// Show what the query finds: the server's answer, once asked, else the
    /// messages listed with every word of it in their sender, subject or
    /// preview.
    pub fn narrow(&mut self) {
        let picked = self.current().map(|m| (m.account, m.folder.clone(), m.uid));
        let words: Vec<String> = self
            .query
            .split_whitespace()
            .map(str::to_lowercase)
            .collect();
        self.shown = match (&self.found, &self.listing) {
            (Some(found), _) if !words.is_empty() => found.clone(),
            (_, Some(l)) => merged(&l.parts)
                .into_iter()
                .filter(|(_, m)| {
                    words.is_empty() || {
                        let text = format!("{} {} {} {}", m.from, m.address, m.subject, m.preview)
                            .to_lowercase();
                        words.iter().all(|w| text.contains(w.as_str()))
                    }
                })
                .map(|(p, m)| Listed::new(p.account, &p.folder, m.clone()))
                .collect(),
            (_, None) => Vec::new(),
        };
        let at = picked
            .and_then(|(a, f, u)| self.shown.iter().position(|m| m.is(a, &f, u)))
            .or(self.list.selected())
            .unwrap_or(0);
        self.list
            .select((!self.shown.is_empty()).then(|| at.min(self.shown.len() - 1)));
    }

    /// Read the message picked in the list: at once, if it was read before.
    fn read_picked(&mut self) {
        let Some(s) = self.current() else { return };
        let (a, folder, uid, seen) = (s.account, s.folder.clone(), s.uid, s.flags.seen);
        if self
            .reading
            .as_ref()
            .is_some_and(|r| r.account == a && r.folder == folder && r.uid == uid)
        {
            return;
        }
        self.scroll = 0;
        let mark = self.mark_read && !seen;
        let body = match self.read.get(&(a, folder.clone(), uid)) {
            Some(m) => {
                if mark {
                    self.set(Flag::Seen, true);
                }
                Body::Ready(m)
            }
            None => {
                self.ask(
                    a,
                    Ask::Open {
                        folder: folder.clone(),
                        uid,
                        mark,
                    },
                );
                Body::Loading
            }
        };
        self.reading = Some(Reading {
            account: a,
            folder,
            uid,
            body,
        });
    }

    /// Mark the message picked, or not, here and on its server.
    fn set(&mut self, flag: Flag, on: bool) {
        let Some(s) = self.current() else { return };
        let (a, folder, uid) = (s.account, s.folder.clone(), s.uid);
        self.marked(a, &folder, uid, flag, on);
        self.ask(
            a,
            Ask::Set {
                folder,
                uid,
                flag,
                on,
            },
        );
    }

    fn toggle(&mut self, flag: Flag) {
        let Some(s) = self.current() else { return };
        let on = match flag {
            Flag::Seen => !s.flags.seen,
            Flag::Flagged => !s.flags.flagged,
            Flag::Answered => !s.flags.answered,
        };
        self.set(flag, on);
    }

    /// The message read, if it has come.
    pub fn message(&self) -> Option<&Arc<Message>> {
        match &self.reading.as_ref()?.body {
            Body::Ready(m) => Some(m),
            _ => None,
        }
    }

    /// Ask for more of the folder's messages, if the one picked is near the
    /// end of those listed and there are more.
    fn more(&mut self) {
        let near = self
            .list
            .selected()
            .is_some_and(|i| i + 10 >= self.shown.len());
        let Some(l) = &mut self.listing else { return };
        if !near || self.found.is_some() || !self.query.is_empty() {
            return;
        }
        // Those listed in part, but for those whose oldest listed is older
        // than the cut: they are not what holds the list back.
        let cut = cut(&l.parts);
        let asks: Vec<(usize, String, usize)> = l
            .parts
            .iter_mut()
            .filter(|p| !p.loading && p.partial())
            .filter(|p| cut.is_none_or(|c| p.oldest().is_none_or(|o| o >= c)))
            .map(|p| {
                p.loading = true;
                (p.account, p.folder.clone(), p.messages.len())
            })
            .collect();
        for (a, folder, offset) in asks {
            self.ask_page(a, folder, offset);
        }
    }

    pub fn key(&mut self, key: KeyEvent) {
        if self.compose.is_some() {
            return self.compose_key(key);
        }
        if key.modifiers.contains(KeyModifiers::CONTROL) && key.code == KeyCode::Char('c') {
            self.quit = true;
            return;
        }
        if self.dialog.is_open() {
            return self.dialog_key(key);
        }
        if self.searching {
            match key.code {
                KeyCode::Esc => {
                    self.query.clear();
                    self.found = None;
                    self.searching = false;
                }
                KeyCode::Enter => {
                    self.searching = false;
                    self.search();
                }
                KeyCode::Down | KeyCode::Up => self.searching = false,
                _ => {
                    if edit_line(&mut self.query, key) {
                        self.found = None;
                    }
                }
            }
            self.narrow();
            return;
        }
        if let Some(act) = keymap::act(key) {
            self.act(act);
        }
    }

    /// Do what a key in the table does, where the focus is.
    fn act(&mut self, act: Act) {
        let page = self.height.saturating_sub(2).max(1) as isize;
        match act {
            Act::Quit => self.quit = true,
            Act::Help => self.dialog = Dialog::Help(0),
            Act::Search => {
                self.searching = true;
                if self.focus == Focus::Side {
                    self.focus = Focus::List;
                }
            }
            Act::Back if self.focus == Focus::Reader => self.focus = Focus::List,
            Act::Back if !self.query.is_empty() => {
                self.query.clear();
                self.found = None;
                self.narrow();
            }
            Act::Back => {}
            Act::NextPane => {
                self.focus = match self.focus {
                    Focus::Side => Focus::List,
                    Focus::List if self.reading.is_some() => Focus::Reader,
                    Focus::List | Focus::Reader => Focus::Side,
                }
            }
            Act::PreviousPane => {
                self.focus = match self.focus {
                    Focus::Side if self.reading.is_some() => Focus::Reader,
                    Focus::Side | Focus::Reader => Focus::List,
                    Focus::List => Focus::Side,
                }
            }
            Act::Left => {
                self.focus = match self.focus {
                    Focus::Reader => Focus::List,
                    _ => Focus::Side,
                }
            }
            Act::Right => match self.focus {
                Focus::Side => {
                    self.open_folder();
                    self.focus = Focus::List;
                }
                _ => {
                    self.read_picked();
                    if self.reading.is_some() {
                        self.focus = Focus::Reader;
                    }
                }
            },
            Act::Open => match self.focus {
                Focus::Side if self.rows.get(self.row) == Some(&Row::Add) => self.open_form(),
                Focus::Side => {
                    self.due = None;
                    self.open_folder();
                    self.focus = Focus::List;
                }
                _ => {
                    self.due = None;
                    self.read_picked();
                    self.focus = Focus::Reader;
                }
            },
            Act::Down => self.step(1),
            Act::Up => self.step(-1),
            Act::PageDown if self.focus == Focus::Side => self.step(5),
            Act::PageUp if self.focus == Focus::Side => self.step(-5),
            Act::ScreenDown | Act::ScreenUp if self.focus == Focus::Side => {}
            Act::PageDown | Act::ScreenDown => self.scroll_by(page),
            Act::PageUp | Act::ScreenUp => self.scroll_by(-page),
            Act::Top => self.jump(false),
            Act::End => self.jump(true),
            Act::ReplyAll => self.respond(Respond::All),
            Act::Reply => self.respond(Respond::Reply),
            Act::Forward => self.respond(Respond::Forward),
            Act::NewMail => {
                self.refresh();
                self.status = "looking for new mail…".to_string();
            }
            Act::NextMessage => self.next(1),
            Act::PreviousMessage => self.next(-1),
            Act::Seen => self.toggle(Flag::Seen),
            Act::Flag => self.toggle(Flag::Flagged),
            Act::ReplyHint => {
                let say = |act| keymap::keys_for(act).join(" or ");
                self.status = format!(
                    "reply is {}, to all {}; {} looks for new mail",
                    say(Act::Reply),
                    say(Act::ReplyAll),
                    say(Act::NewMail)
                );
            }
            Act::Browser => self.open_in_browser(),
            Act::SaveFiles => self.save_attachments(),
            Act::Files => self.list_files(),
            Act::AddAccount => self.open_form(),
            Act::Remove => self.ask_to_remove(),
            Act::Trash => self.delete(),
            Act::Compose => self.compose_new(),
            Act::Rename => self.ask_to_rename(),
        }
    }

    /// Take a key in the dialog open.
    fn dialog_key(&mut self, key: KeyEvent) {
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
    fn dialog_click(&mut self, at: Option<Target>) {
        match (&mut self.dialog, at) {
            (_, Some(Target::Popup)) => {}
            // A key at the foot, pressed.
            (_, Some(Target::Key(key))) => self.dialog_key(key),
            (Dialog::Closed, _) => {}
            // Away from help, or from renaming, closes it, the name left
            // as it is.
            (Dialog::Help(_) | Dialog::Rename(..), _) => self.dialog = Dialog::Closed,
            (Dialog::Trash(pick), Some(Target::Item(i))) => {
                pick.at = i;
                self.choose_trash();
            }
            (Dialog::Trash(_), _) => self.cancel_trash(),
            (Dialog::Files(files), Some(Target::Item(i))) => {
                files.at = i;
                self.open_item();
            }
            (Dialog::Files(_), _) => self.dialog = Dialog::Closed,
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
        }
    }

    /// The account `D` and `R` act on: the one picked in the sidebar, or,
    /// with the focus elsewhere, the one listed, else the message's.
    fn account_picked(&self) -> Option<usize> {
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
    fn ask_to_remove(&mut self) {
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
    fn ask_to_rename(&mut self) {
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
    fn rename(&mut self, a: usize, name: &str) {
        let old = self.accounts[a].name.clone();
        match config::rename(&self.config, &old, name) {
            Ok(name) => {
                self.status = format!("renamed {old} to {name}");
                self.accounts[a].name = name;
            }
            Err(e) => self.status = format!("cannot rename {old}: {e:#}"),
        }
    }

    fn search(&mut self) {
        let Some(l) = &self.listing else { return };
        if self.query.trim().is_empty() {
            return;
        }
        let parts = l.keys();
        self.status = format!("searching {} on the server…", self.folder_name());
        for (a, folder) in parts {
            self.ask_search(a, folder);
        }
    }

    /// Move through the sidebar or the list, or scroll the message read.
    fn step(&mut self, by: isize) {
        match self.focus {
            Focus::Side => {
                let mut at = self.row;
                loop {
                    let next = at.saturating_add_signed(by.signum());
                    if next == at || next >= self.rows.len() {
                        break;
                    }
                    at = next;
                    if matches!(self.rows[at], Row::All(_) | Row::Folder(..) | Row::Add) {
                        self.row = at;
                        break;
                    }
                }
                self.due = Some((Instant::now(), Due::Folder));
            }
            Focus::List => self.pick(by),
            Focus::Reader => self.scroll_by(by),
        }
    }

    /// Pick another message in the list, `by` from the one picked, and read
    /// it once moving pauses, if it is shown beside the list.
    fn pick(&mut self, by: isize) {
        if self.shown.is_empty() {
            return;
        }
        let at = self.list.selected().unwrap_or(0);
        self.list
            .select(Some(at.saturating_add_signed(by).min(self.shown.len() - 1)));
        if self.side_by_side {
            self.due = Some((Instant::now(), Due::Message));
        }
        self.more();
    }

    /// Read the next message in the list, or the one before.
    fn next(&mut self, by: isize) {
        if self.shown.is_empty() {
            return;
        }
        let at = self.list.selected().unwrap_or(0);
        self.list
            .select(Some(at.saturating_add_signed(by).min(self.shown.len() - 1)));
        self.due = None;
        self.read_picked();
        self.more();
    }

    fn scroll_by(&mut self, by: isize) {
        if self.reading.is_none() {
            return self.pick(by.signum());
        }
        let last = self.lines.saturating_sub(self.height);
        self.scroll = self.scroll.saturating_add_signed(by).min(last);
    }

    fn jump(&mut self, end: bool) {
        match self.focus {
            Focus::Reader => {
                self.scroll = if end {
                    self.lines.saturating_sub(self.height)
                } else {
                    0
                }
            }
            Focus::List if !self.shown.is_empty() => {
                self.list
                    .select(Some(if end { self.shown.len() - 1 } else { 0 }));
                if self.side_by_side {
                    self.due = Some((Instant::now(), Due::Message));
                }
                self.more();
            }
            _ => {}
        }
    }

    /// Take a click or a turn of the wheel at where it lands.
    pub fn mouse(&mut self, m: MouseEvent) {
        if self.compose.is_some() {
            return;
        }
        let at = self
            .targets
            .iter()
            .rev()
            .find(|(r, _)| r.contains(Position::new(m.column, m.row)))
            .map(|&(_, t)| t);
        let by = match m.kind {
            MouseEventKind::Down(MouseButton::Left) => return self.click(at),
            MouseEventKind::ScrollDown => 1,
            MouseEventKind::ScrollUp => -1,
            _ => return,
        };
        // A dialog open, nothing behind it scrolls.
        if self.dialog.is_open() {
            return self.dialog.step(by);
        }
        match at {
            Some(Target::Side | Target::Row(_)) => {
                let before = self.focus;
                self.focus = Focus::Side;
                self.step(by);
                self.focus = before;
            }
            Some(Target::List | Target::Message(_)) => self.pick(by),
            Some(Target::Reader) => self.scroll_by(3 * by),
            _ => {}
        }
    }

    fn click(&mut self, at: Option<Target>) {
        if self.dialog.is_open() {
            return self.dialog_click(at);
        }
        self.searching = false;
        match at {
            Some(Target::Key(key)) => self.key(key),
            Some(Target::Search) => self.searching = true,
            Some(Target::Side) => self.focus = Focus::Side,
            Some(Target::Row(i)) if self.rows.get(i) == Some(&Row::Add) => {
                self.row = i;
                self.open_form();
            }
            Some(Target::Row(i)) => {
                self.focus = Focus::Side;
                if matches!(self.rows.get(i), Some(Row::Folder(..) | Row::All(_))) {
                    self.row = i;
                    self.open_folder();
                    self.focus = Focus::List;
                }
            }
            Some(Target::List) => self.focus = Focus::List,
            Some(Target::Message(i)) => {
                self.focus = Focus::List;
                self.list.select(Some(i));
                let again = self
                    .clicked
                    .is_some_and(|(when, j)| j == i && when.elapsed() < DOUBLE_CLICK);
                self.clicked = (!again).then(|| (Instant::now(), i));
                self.due = None;
                if self.side_by_side || again {
                    self.read_picked();
                }
                if again || !self.side_by_side {
                    self.focus = Focus::Reader;
                }
                self.more();
            }
            Some(Target::Reader) if self.reading.is_some() => self.focus = Focus::Reader,
            _ => {}
        }
    }

    /// Show the message read in the browser: its HTML, or its text.
    fn open_in_browser(&mut self) {
        let Some(m) = self.message().cloned() else {
            self.status = "nothing to open: ⏎ reads the message picked".to_string();
            return;
        };
        let body = m.html.clone().unwrap_or_else(|| {
            format!(
                "<meta charset=utf-8><pre style='white-space: pre-wrap'>{}</pre>",
                escape(&m.text)
            )
        });
        // Mail is from strangers: its scripts, plugins and forms are not run
        // from a file that could reach this machine's other files.
        let html = format!(
            "<meta http-equiv=\"Content-Security-Policy\" content=\"script-src 'none'; \
             object-src 'none'; base-uri 'none'; form-action 'none'\">{body}"
        );
        let dir = cache().join("html");
        let path = dir.join(format!("{}.html", m.uid));
        let done = private_dir(&dir)
            .map_err(anyhow::Error::from)
            .and_then(|_| vault::write_private(&path, html.as_bytes()))
            .and_then(|_| open(&path));
        self.status = match done {
            Ok(()) => format!("opened {}", path.display()),
            Err(e) => format!("cannot open it: {e:#}"),
        };
    }

    /// Save the message's attachments to the downloads folder, each under
    /// a name not yet taken there.
    /// List the attachments and links of the message read, to open.
    fn list_files(&mut self) {
        let Some(m) = self.message().cloned() else {
            self.status = "nothing to list: ⏎ reads the message picked".to_string();
            return;
        };
        let mut items: Vec<Item> = m
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
        items.extend(m.links.iter().cloned().map(Item::Link));
        if items.is_empty() {
            self.status = "no attachments or links".to_string();
            return;
        }
        self.dialog = Dialog::Files(Files {
            items,
            at: 0,
            note: None,
        });
    }

    fn files_key(&mut self, key: KeyEvent) {
        let Some(files) = self.dialog.files_mut() else {
            return;
        };
        match (keymap::act(key), key.code) {
            (Some(Act::Down), _) => self.dialog.step(1),
            (Some(Act::Up), _) => self.dialog.step(-1),
            (Some(Act::Open), _) | (_, KeyCode::Char('o')) => self.open_item(),
            (_, KeyCode::Char('s')) => self.save_item(),
            (_, KeyCode::Char('y')) => {
                if let Some(Item::Link(url)) = files.items.get(files.at) {
                    self.launch = Some(Launch::Copy(url.clone()));
                }
            }
            _ => self.dialog = Dialog::Closed,
        }
    }

    /// Open the attachment or link picked: in ohp, a PDF or markdown, a
    /// linked PDF downloaded first; else by the system, or in the browser.
    fn open_item(&mut self) {
        let m = self.message().cloned();
        let (Some(files), Some(m)) = (self.dialog.files_mut(), m) else {
            return;
        };
        match files.items.get(files.at).cloned() {
            Some(Item::File { at, name, .. }) => {
                let Some((_, bytes)) = mail::attachments(&m.raw).into_iter().nth(at) else {
                    return;
                };
                match keep_file(m.uid, &name, &bytes) {
                    Ok(path) if ohp_reads(&name) => self.launch = Some(Launch::Ohp(path)),
                    Ok(path) => self.launch = Some(Launch::Open(path)),
                    Err(e) => files.note = Some(format!("cannot keep {name}: {e:#}")),
                }
            }
            Some(Item::Link(url)) => match crate::net::pdf_of(&url) {
                Some(pdf) => {
                    files.note = Some(format!("downloading {pdf}…"));
                    let (replies, uid) = (self.replies_to.clone(), m.uid);
                    self.pending += 1;
                    std::thread::spawn(move || {
                        let got = crate::net::pdf(&pdf)
                            .and_then(|(name, bytes)| keep_file(uid, &name, &bytes))
                            .map_err(worker::text);
                        let _ = replies.send(Reply {
                            account: usize::MAX,
                            answer: Answer::Fetched(got),
                        });
                    });
                }
                None => self.launch = Some(Launch::Browse(url)),
            },
            None => {}
        }
    }

    /// Save the attachment picked to the downloads folder, under a name not
    /// yet taken there.
    fn save_item(&mut self) {
        let m = self.message().cloned();
        let (Some(files), Some(m)) = (self.dialog.files_mut(), m) else {
            return;
        };
        let Some(Item::File { at, name, .. }) = files.items.get(files.at).cloned() else {
            files.note = Some("a link is opened, not saved: ⏎".to_string());
            return;
        };
        let Some((_, bytes)) = mail::attachments(&m.raw).into_iter().nth(at) else {
            return;
        };
        let dir = downloads();
        let path = unused(&dir, &safe_name(&name));
        files.note = Some(
            match std::fs::create_dir_all(&dir).and_then(|_| write_new(&path, &bytes)) {
                Ok(()) => format!("saved {}", tilde(&path)),
                Err(e) => format!("cannot save {name}: {e}"),
            },
        );
    }

    fn save_attachments(&mut self) {
        let Some(m) = self.message().cloned() else {
            self.status = "nothing to save: ⏎ reads the message picked".to_string();
            return;
        };
        let files = mail::attachments(&m.raw);
        if files.is_empty() {
            self.status = "no attachments".to_string();
            return;
        }
        let dir = downloads();
        let mut saved = Vec::new();
        for (Attachment { name, .. }, bytes) in files {
            let path = unused(&dir, &safe_name(&name));
            match std::fs::create_dir_all(&dir).and_then(|_| write_new(&path, &bytes)) {
                Ok(()) => saved.push(
                    path.file_name()
                        .map(|n| n.to_string_lossy().into_owned())
                        .unwrap_or_default(),
                ),
                Err(e) => {
                    self.status = format!("cannot save {name}: {e}");
                    return;
                }
            }
        }
        self.status = format!("saved to {}: {}", tilde(&dir), saved.join(", "));
    }

    /// Move what is picked to Trash, or, in Trash, back where it was.
    fn delete(&mut self) {
        let Some(Listed {
            account,
            folder,
            summary,
        }) = self.current().cloned()
        else {
            return;
        };
        let uid = summary.uid;
        if self.is_trash(account, &folder) {
            let email = self.accounts[account].email.clone();
            let known = self
                .ledger
                .origin(&email, &folder, &summary.message_id, uid);
            let dest = known.clone().unwrap_or_else(|| self.inbox(account));
            let name = self.folder_label(account, &dest);
            self.drop_listed(account, &folder, uid);
            self.status = if known.is_some() {
                format!("put back in {name}")
            } else {
                format!("put back in {name}; letterbox had not moved it")
            };
            self.ask(
                account,
                Ask::Move {
                    from: folder,
                    uid,
                    to: dest,
                    message_id: summary.message_id,
                    trashed: false,
                },
            );
            return;
        }
        let Some(trash) = self.trash_of(account) else {
            self.ask_which_trash(account, folder, summary);
            return;
        };
        self.move_to_trash(account, folder, summary.uid, summary.message_id, trash);
    }

    /// List the account's other folders and ask which one `d` should move
    /// mail to. A server that marks a Trash folder is not asked.
    fn ask_which_trash(&mut self, account: usize, folder: String, summary: Summary) {
        let folders: Vec<(String, String)> = self
            .accounts
            .get(account)
            .map(|a| {
                a.folders
                    .iter()
                    .filter(|f| f.selectable && f.path != folder)
                    .map(|f| (f.path.clone(), trash_label(f)))
                    .collect()
            })
            .unwrap_or_default();
        if folders.is_empty() {
            self.status = "there is no other folder to use as Trash".to_string();
            return;
        }
        self.held = Some(Held {
            account,
            from: folder,
            uid: summary.uid,
            message_id: summary.message_id,
        });
        self.dialog = Dialog::Trash(TrashPick {
            account,
            folders,
            at: 0,
        });
        self.status = "which folder should be Trash?".to_string();
    }

    fn cancel_trash(&mut self) {
        self.dialog = Dialog::Closed;
        self.held = None;
        self.status = "not moved".to_string();
    }

    /// The folder highlighted is Trash from now on, and the delete that
    /// was waiting is moved there.
    fn choose_trash(&mut self) {
        let Dialog::Trash(pick) = std::mem::take(&mut self.dialog) else {
            return;
        };
        let Some((path, _)) = pick.folders.get(pick.at).cloned() else {
            self.held = None;
            self.status = "not moved".to_string();
            return;
        };
        if let Some(folder) = self.folder_mut(pick.account, &path) {
            folder.role = Role::Trash;
        }
        self.accounts[pick.account].trash = Some(path.clone());
        let remembered = if self.editable {
            let name = self.accounts[pick.account].name.clone();
            config::set_trash(&self.config, &name, &path)
        } else {
            Ok(())
        };
        let moved = self.resume_held(pick.account);
        if let Err(e) = remembered
            && moved
        {
            self.status = format!(
                "{} · not remembered: {}",
                self.status,
                first_line(&format!("{e:#}"))
            );
        }
    }

    fn move_to_trash(
        &mut self,
        account: usize,
        from: String,
        uid: u32,
        message_id: String,
        trash: String,
    ) {
        let name = self.folder_label(account, &trash);
        self.drop_listed(account, &from, uid);
        self.status = format!("moved to {name} · d there puts it back, for 7 days");
        self.ask(
            account,
            Ask::Move {
                from,
                uid,
                to: trash,
                message_id,
                trashed: true,
            },
        );
    }

    /// Move the delete that was waiting. False when there was nothing to
    /// move, or Trash is still not known.
    fn resume_held(&mut self, account: usize) -> bool {
        let Some(held) = self.held.take() else {
            return false;
        };
        if held.account != account {
            self.held = Some(held);
            return false;
        }
        let Some(trash) = self.trash_of(account) else {
            // Still waiting on which folder that is.
            if matches!(self.dialog, Dialog::Trash(_)) {
                self.held = Some(held);
            } else {
                self.status = "no Trash folder".to_string();
            }
            return false;
        };
        self.move_to_trash(account, held.from, held.uid, held.message_id, trash);
        true
    }

    /// The folder mail is moved to: the one chosen for this account, when
    /// it is still there, else the one the server marks as Trash.
    fn trash_of(&self, account: usize) -> Option<String> {
        let account = self.accounts.get(account)?;
        if let Some(path) = &account.trash
            && account
                .folders
                .iter()
                .any(|f| f.path == *path && f.selectable)
        {
            return Some(path.clone());
        }
        account
            .folders
            .iter()
            .find(|f| f.role == Role::Trash && f.selectable)
            .map(|f| f.path.clone())
    }

    /// Give the chosen folder the Trash role, once the server has listed it.
    fn remember_trash(&mut self, account: usize) {
        let Some(path) = self.accounts[account].trash.clone() else {
            return;
        };
        if let Some(folder) = self.folder_mut(account, &path)
            && folder.selectable
        {
            folder.role = Role::Trash;
        }
    }

    fn inbox(&self, account: usize) -> String {
        self.accounts
            .get(account)
            .and_then(|a| {
                a.folders
                    .iter()
                    .find(|f| f.role == Role::Inbox)
                    .map(|f| f.path.clone())
            })
            .unwrap_or_else(|| "INBOX".to_string())
    }

    fn folder_label(&self, account: usize, path: &str) -> String {
        self.folder_of(account, path)
            .map(|f| f.name.clone())
            .unwrap_or_else(|| path.to_string())
    }

    /// Remove messages letterbox trashed more than seven days ago.
    fn sweep(&mut self, account: usize) {
        let email = self
            .accounts
            .get(account)
            .map(|a| a.email.clone())
            .unwrap_or_default();
        if email.is_empty() {
            return;
        }
        let now = jiff::Timestamp::now().as_second();
        let due = self.ledger.due(&email, now);
        for entry in due {
            let key = (email.clone(), entry.folder.clone(), entry.uid);
            if !self.purging.insert(key) {
                continue;
            }
            self.ask(
                account,
                Ask::Expunge {
                    folder: entry.folder,
                    uid: entry.uid,
                },
            );
        }
    }

    fn forget_trashed(&mut self, email: &str, folder: &str, uid: u32) {
        if let Err(e) = self.ledger.forget(email, folder, uid) {
            self.status = format!(
                "cannot update the trash record: {}",
                first_line(&format!("{e:#}"))
            );
        }
    }

    fn drop_listed(&mut self, account: usize, folder: &str, uid: u32) {
        if let Some(listing) = self
            .listing
            .as_mut()
            .and_then(|l| l.part_mut(account, folder))
        {
            let before = listing.messages.len();
            listing.messages.retain(|m| m.uid != uid);
            if listing.messages.len() != before {
                listing.total = listing.total.saturating_sub(1);
            }
        }
        if let Some((messages, total)) = self.listed.get_mut(&(account, folder.to_string())) {
            let before = messages.len();
            messages.retain(|m| m.uid != uid);
            if messages.len() != before {
                *total = total.saturating_sub(1);
            }
        }
        if self
            .reading
            .as_ref()
            .is_some_and(|r| r.account == account && r.folder == folder && r.uid == uid)
        {
            self.reading = None;
            self.drawn = None;
        }
        self.narrow();
    }

    fn reload(&mut self, account: usize) {
        let folders = self.listing.as_ref().map(Listing::keys).unwrap_or_default();
        for (_, folder) in folders.into_iter().filter(|(a, _)| *a == account) {
            self.ask_page(account, folder, 0);
        }
    }

    fn compose_new(&mut self) {
        let Some(account) = self.account_of() else {
            self.status = "no account to send from".to_string();
            return;
        };
        let (text, row, col) = compose::blank();
        self.open_compose(account, text, (row, col), Thread::default(), Vec::new());
    }

    /// Answer the message picked, as `how` says; read first, if it was
    /// not.
    fn respond(&mut self, how: Respond) {
        let Some(summary) = self.current() else {
            self.status = "no message to answer".to_string();
            return;
        };
        let (account, folder, uid) = (summary.account, summary.folder.clone(), summary.uid);
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

    fn cached(&self, account: usize, folder: &str, uid: u32) -> Option<std::sync::Arc<Message>> {
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
    fn open_reply(&mut self, account: usize, folder: &str, message: &Message, how: Respond) {
        let me = self.accounts[account].email.clone();
        let (text, at, thread, files) = compose::respond(message, folder, how, &me);
        self.open_compose(account, text, at, thread, files);
    }

    fn open_compose(
        &mut self,
        account: usize,
        text: String,
        (row, col): (usize, usize),
        thread: Thread,
        files: Vec<compose::File>,
    ) {
        let mut buffer = edit::Buffer::from_text(&text);
        buffer.place(row, col);
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
        });
    }

    /// The account to write from: the one listed, else the one of the
    /// message picked, else the first.
    fn account_of(&self) -> Option<usize> {
        self.listing
            .as_ref()
            .and_then(Listing::account)
            .or_else(|| self.current().map(|m| m.account))
            .or_else(|| self.live().next())
    }

    fn compose_key(&mut self, key: KeyEvent) {
        let Some(compose) = &mut self.compose else {
            return;
        };
        if compose.sending {
            return;
        }
        let done = edit::handle(&mut compose.buffer, &mut compose.prefix, key);
        match done {
            Done::Stay => {}
            Done::Cancel => {
                self.compose = None;
                self.status = "not sent".to_string();
            }
            Done::Send => self.send_compose(),
        }
    }

    fn send_compose(&mut self) {
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
        let raw = compose::bytes(
            &email,
            &draft,
            compose.thread.in_reply_to.as_deref(),
            compose.thread.references.as_deref(),
            &compose.files,
        );
        let answer = compose.thread.answering.clone();
        self.compose.as_mut().expect("open").sending = true;
        self.status = "sending…".to_string();
        self.ask(account, Ask::Send { raw, answer });
    }
}

/// Edit a line typed in, a field's or the search's, by `key`: a character
/// typed, `⌫` the last taken back, `C-u` all of it. Whether it changed.
fn edit_line(line: &mut String, key: KeyEvent) -> bool {
    let ctrl = key.modifiers.contains(KeyModifiers::CONTROL);
    match key.code {
        KeyCode::Backspace => line.pop().is_some(),
        KeyCode::Char('u') if ctrl => {
            let had = !line.is_empty();
            line.clear();
            had
        }
        KeyCode::Char(c) if !ctrl => {
            line.push(c);
            true
        }
        _ => false,
    }
}

/// How a folder is named when asking which one is Trash: its own name, and
/// its path when that says where it sits.
fn trash_label(folder: &Folder) -> String {
    if folder.path.eq_ignore_ascii_case(&folder.name) || folder.role == Role::Inbox {
        folder.name.clone()
    } else {
        format!("{} · {}", folder.name, folder.path)
    }
}

fn first_line(s: &str) -> &str {
    s.lines().next().unwrap_or_default()
}

fn escape(s: &str) -> String {
    s.replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
}

fn open(path: &std::path::Path) -> anyhow::Result<()> {
    let program = if cfg!(target_os = "macos") {
        "open"
    } else {
        "xdg-open"
    };
    let status = std::process::Command::new(program)
        .arg(path)
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .status()?;
    if !status.success() {
        anyhow::bail!("{program}: {status}");
    }
    Ok(())
}

/// Keep `bytes`, an attachment or a document downloaded, of the message
/// `uid`, in a private folder of the cache, to be opened from there.
fn keep_file(uid: u32, name: &str, bytes: &[u8]) -> anyhow::Result<PathBuf> {
    let dir = cache().join("files");
    private_dir(&dir)?;
    let path = dir.join(format!("{uid}-{}", safe_name(name)));
    vault::write_private(&path, bytes)?;
    Ok(path)
}

/// `$LETTERBOX_DOWNLOADS`, else `~/Downloads`.
fn downloads() -> PathBuf {
    std::env::var_os("LETTERBOX_DOWNLOADS")
        .filter(|d| !d.is_empty())
        .map(PathBuf::from)
        .unwrap_or_else(|| {
            PathBuf::from(std::env::var_os("HOME").unwrap_or_default()).join("Downloads")
        })
}

/// `$XDG_CACHE_HOME/letterbox`, else `~/.cache/letterbox`: in your home,
/// not the temporary folder others on the machine may write in too.
fn cache() -> PathBuf {
    std::env::var_os("XDG_CACHE_HOME")
        .filter(|d| !d.is_empty())
        .map(PathBuf::from)
        .unwrap_or_else(|| {
            PathBuf::from(std::env::var_os("HOME").unwrap_or_default()).join(".cache")
        })
        .join("letterbox")
}

/// `dir`, made if it is not there, only you may look in, where it is made.
fn private_dir(dir: &std::path::Path) -> std::io::Result<()> {
    let mut builder = std::fs::DirBuilder::new();
    builder.recursive(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::DirBuilderExt;
        builder.mode(0o700);
    }
    builder.create(dir)
}

/// Write `bytes` to a file not yet at `path`: nothing there is written
/// over, nor a link there followed.
fn write_new(path: &std::path::Path, bytes: &[u8]) -> std::io::Result<()> {
    use std::io::Write;
    std::fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(path)?
        .write_all(bytes)
}

/// An attachment's name as a file's: no folders in it, as a sender might
/// try, nothing hidden, and none of the marks that turn text around, as
/// `invoice\u{202e}fdp.exe` shown as `invoiceexe.pdf`.
fn safe_name(name: &str) -> String {
    let folders: &[char] = if cfg!(windows) {
        &['/', '\\', ':']
    } else {
        &['/', '\\']
    };
    let base = name.rsplit(folders).next().unwrap_or_default();
    let base: String = base
        .chars()
        .filter(|c| {
            !matches!(
                c,
                '\u{200e}' | '\u{200f}' | '\u{061c}' | '\u{202a}'..='\u{202e}' | '\u{2066}'..='\u{2069}'
            )
        })
        .collect();
    let base = base.trim_start_matches('.').trim();
    if base.is_empty() {
        "attachment".to_string()
    } else {
        base.to_string()
    }
}

/// `dir/name`, or `dir/name (2)`, and so on, the first not there yet.
fn unused(dir: &std::path::Path, name: &str) -> PathBuf {
    let path = dir.join(name);
    if !path.exists() {
        return path;
    }
    let (stem, ext) = match name.rsplit_once('.') {
        Some((s, e)) if !s.is_empty() => (s, format!(".{e}")),
        _ => (name, String::new()),
    };
    (2..)
        .map(|n| dir.join(format!("{stem} ({n}){ext}")))
        .find(|p| !p.exists())
        .expect("a name not taken")
}

fn tilde(path: &std::path::Path) -> String {
    let home = PathBuf::from(std::env::var_os("HOME").unwrap_or_default());
    match path.strip_prefix(&home) {
        Ok(rest) if !home.as_os_str().is_empty() => format!("~/{}", rest.display()),
        _ => path.display().to_string(),
    }
}

/// The screen as `app` would draw it on a terminal `width` by `height`, as
/// text, for tests and `letterbox screen`.
pub fn screen(app: &mut App, width: u16, height: u16) -> String {
    let mut terminal = ratatui::Terminal::new(ratatui::backend::TestBackend::new(width, height))
        .expect("a screen in memory");
    terminal
        .draw(|f| ui::draw(f, app))
        .expect("drawn in memory");
    let buffer = terminal.backend().buffer();
    (0..height)
        .map(|y| {
            (0..width)
                .map(|x| buffer[(x, y)].symbol())
                .collect::<String>()
                .trim_end()
                .to_string()
        })
        .collect::<Vec<_>>()
        .join("\n")
}

/// The screen as an SVG, with its colours, as a terminal would show it:
/// for a look at a theme, or a picture for the README.
pub fn screen_svg(app: &mut App, width: u16, height: u16) -> String {
    use ratatui::style::{Color, Modifier};
    use std::fmt::Write;
    const CW: f64 = 8.4;
    const CH: f64 = 18.0;
    let mut terminal = ratatui::Terminal::new(ratatui::backend::TestBackend::new(width, height))
        .expect("a screen in memory");
    terminal
        .draw(|f| ui::draw(f, app))
        .expect("drawn in memory");
    let buffer = terminal.backend().buffer();
    let dark = matches!(app.theme.page, Color::Reset);
    let hex = |c: Color, bg: bool| -> String {
        let (r, g, b) = match c {
            Color::Rgb(r, g, b) => (r, g, b),
            Color::Reset if bg => {
                if dark {
                    (28, 28, 28)
                } else {
                    (255, 255, 255)
                }
            }
            Color::Reset => {
                if dark {
                    (222, 222, 222)
                } else {
                    (40, 40, 40)
                }
            }
            Color::Indexed(n) if n >= 232 => {
                let v = 8 + (n - 232) * 10;
                (v, v, v)
            }
            Color::Indexed(n) if n >= 16 => {
                let level = |x: u8| if x == 0 { 0 } else { 55 + x * 40 };
                let n = n - 16;
                (level(n / 36), level(n / 6 % 6), level(n % 6))
            }
            _ => (128, 128, 128),
        };
        format!("#{r:02x}{g:02x}{b:02x}")
    };
    let mut svg = format!(
        "<svg xmlns='http://www.w3.org/2000/svg' width='{}' height='{}' \
         font-family='Menlo, Monaco, monospace' font-size='14'>\n",
        f64::from(width) * CW,
        f64::from(height) * CH
    );
    for y in 0..height {
        for x in 0..width {
            let cell = &buffer[(x, y)];
            let (px, py) = (f64::from(x) * CW, f64::from(y) * CH);
            let _ = writeln!(
                svg,
                "<rect x='{px:.1}' y='{py:.1}' width='{:.1}' height='{CH}' fill='{}'/>",
                CW + 0.6,
                hex(cell.bg, true)
            );
            let symbol = cell.symbol();
            if !symbol.trim().is_empty() {
                let bold = if cell.modifier.contains(Modifier::BOLD) {
                    " font-weight='bold'"
                } else {
                    ""
                };
                let text = symbol
                    .replace('&', "&amp;")
                    .replace('<', "&lt;")
                    .replace('>', "&gt;");
                let _ = writeln!(
                    svg,
                    "<text x='{px:.1}' y='{:.1}' fill='{}'{bold}>{text}</text>",
                    py + 13.5,
                    hex(cell.fg, false)
                );
            }
        }
    }
    svg.push_str("</svg>\n");
    svg
}

/// A key as `letterbox screen` is given it: `<enter>`, `<esc>`, `<tab>`,
/// `<space>`, `<up>`, `<down>`, `<left>`, `<right>`, `<backspace>`, and
/// with control, as `<c-u>`; anything else, each character typed.
pub fn keys(spec: &str) -> Vec<KeyEvent> {
    // `<c-s-r>` is `C-S-r`, `<enter>` is `enter`, as the foot names them.
    if let Some(name) = spec.strip_prefix('<').and_then(|s| s.strip_suffix('>'))
        && let Some(key) = keymap::key_named(
            &name
                .replace("c-", "C-")
                .replace("s-", "S-")
                .replace("m-", "M-"),
        )
    {
        return vec![key];
    }
    spec.chars()
        .map(|c| KeyEvent::new(KeyCode::Char(c), KeyModifiers::NONE))
        .collect()
}
