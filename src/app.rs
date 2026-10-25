//! The TUI: what is shown, and what keys do. Each account's store is asked
//! from its own thread; what it answers is taken in between keys, so the
//! screen is never kept waiting by a server.

use crate::compose;
use crate::compose::{Respond, Thread};
use crate::config::{self, Auth};
use crate::edit::{self, Done};
use crate::index::{Index, sync};
use crate::keymap::{self, Act};
use crate::mail::{self, Attachment, Folder, Message, Role, Summary};
use crate::map::{self, Focused, Map, Mode};
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
    self, DisableBracketedPaste, DisableMouseCapture, EnableBracketedPaste, EnableMouseCapture,
    Event, KeyCode, KeyEvent, KeyEventKind, KeyModifiers, MouseButton, MouseEvent, MouseEventKind,
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
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{self, Receiver, Sender};
use std::sync::{Arc, Mutex};
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
    /// What its store is known by in the index.
    pub ident: String,
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
    /// The map's heading, the map's every cluster, each cluster by its
    /// number, and the messages that fit nowhere.
    MapHead,
    Map,
    Cluster(u32),
    Loners,
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
    /// The map's clusters, every one, or one; or what fits nowhere.
    Map,
    Cluster(u32),
    Loners,
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
    /// What the map lists, as its snapshot has it, in its order: not asked
    /// of the servers, which keep no clusters.
    pub(crate) fixed: Option<Vec<Listed>>,
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
            _ => None,
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
}

/// What the message read has to open: an attachment, by its place among
/// its attachments, or a link.
#[derive(Clone, Debug, PartialEq)]
pub enum Item {
    /// The message itself, as its HTML, in the browser.
    Page,
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
    /// The files of the message it forwards, sent with it while an
    /// `Attach` line names them.
    pub files: Vec<compose::File>,
    /// Said that what is attached is more than most servers take: a send
    /// asked again sends it.
    pub warned: bool,
    /// The text it was opened with, to tell whether it has been changed.
    original: String,
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
const QUIT: &str = "Quit · the message stays: C-c C-c sends it, C-c C-k discards it";

/// How much attached most servers take, in bytes.
const LARGE: usize = 25 * 1024 * 1024;

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
/// The Inbox looked through beside the map, each message given what to do
/// with it: what is suggested, unless another was chosen; all of it done at
/// once, when asked.
#[derive(Default)]
pub struct Review {
    /// The message picked in it.
    pub picked: Option<crate::index::Key>,
    /// What was chosen for each message, where it is not what is suggested.
    pub chosen: std::collections::BTreeMap<crate::index::Key, Choice>,
    /// The first message shown.
    pub top: usize,
    /// Whether the message picked is read in its place.
    pub reading: bool,
    /// Whether it is asked whether to do it all.
    pub asking: bool,
    /// The message whose label is being picked on the map.
    pub relabel: Option<crate::index::Key>,
    /// The labels it may be moved to, dropped down under it.
    pub menu: Option<Menu>,
}

/// The labels a message looked through may be moved to, dropped down
/// under it: narrowed by what is typed, the one picked lit.
#[derive(Clone, Debug, Default)]
pub struct Menu {
    pub key: crate::index::Key,
    pub typed: String,
    pub at: usize,
}

/// How many labels are dropped down: the nearest few, or more of those
/// holding what is typed.
const MENU_NEAREST: usize = 5;
const MENU_TYPED: usize = 8;

/// What may be picked in the labels dropped down.
#[derive(Clone, Debug, PartialEq)]
pub enum Pick {
    /// A label, and how far it is from the message by angle.
    Label(String, f32),
    /// A label not yet made, as typed.
    New(String),
    /// Where it goes, picked on the map.
    OnMap,
}

/// What to do with a message in the Inbox.
#[derive(Clone, Debug, PartialEq)]
pub enum Choice {
    /// Filed under the label, out of the Inbox.
    Move(String),
    Keep,
    /// To the Trash.
    Discard,
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
    /// What the map shows, and how, to be changed: `keymap::VIEW`.
    View,
    /// Whether to begin the map, asked the first time it is wanted.
    StartMap,
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
            Dialog::Attach(picker) => return picker.step(by),
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
    /// The index the map reads, shared with each account's sync.
    pub index: Arc<Mutex<Index>>,
    /// Why there is no map, if there is none: as its key could not be had.
    pub map_off: Option<String>,
    /// The map is not begun: no key was ever made for it. It is begun once
    /// it is asked for, and agreed to.
    pub map_later: bool,
    /// The Inbox, being looked through beside the map.
    pub review: Option<Review>,
    /// What was chosen in it, kept while it is closed, till it is done.
    pub review_kept: std::collections::BTreeMap<crate::index::Key, Choice>,
    /// Each account's second connection, kept for its sync while the map
    /// is not begun.
    twins: Vec<Option<Box<dyn Store>>>,
    /// How the map's index is made and opened, once it is begun.
    pub open_map: fn() -> Result<Index, String>,
    /// Each account's sync, by the account's place, and what stops its
    /// pass; none once the account is removed.
    syncs: Vec<Option<(Sender<sync::Ask>, Arc<AtomicBool>)>>,
    /// How far the map's sync has got, as it last said.
    pub mapping: String,
    /// The map, as it is looked at.
    pub map: Map,
    /// What the map's model has read, between one snapshot and the next.
    map_corpus: Option<crate::model::Corpus>,
    /// The folder files were last attached from.
    pub attach_dir: Option<PathBuf>,
    /// How wide the message read is, as it was dragged; else as the
    /// screen's width has it.
    pub reader_width: Option<u16>,
    /// The line before the message read is being dragged.
    pub resizing: bool,
    /// Where the panes end on the right, as last drawn.
    pub panes_right: u16,
    /// How wide the message read was last drawn.
    pub read_width: u16,
    /// Where it is written down how the mail was looked at, the list or
    /// the map, to open as it was left; none to keep nothing.
    pub look_file: Option<PathBuf>,
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
                Event::Mouse(m) => {
                    app.mouse(m);
                    // A box being drawn on the map is drawn as it goes.
                    stale |= matches!(m.kind, MouseEventKind::Drag(_))
                        && (app.map.drag.is_some() || app.resizing);
                }
                Event::Paste(text) => app.paste(&text),
                _ => {}
            }
        }
        Ok(())
    })();
    give_back();
    if app.map.mode == Mode::Map {
        app.looked_now();
    }
    app.remember_look();
    let written = app.index.lock().unwrap_or_else(|e| e.into_inner()).flush();
    result.and(written)
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
/// what is pasted as one, not as keys, and keys told apart where the
/// terminal can.
fn take() -> DefaultTerminal {
    let terminal = ratatui::init();
    let _ = execute!(std::io::stdout(), EnableMouseCapture, EnableBracketedPaste);
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
    let _ = execute!(
        std::io::stdout(),
        DisableBracketedPaste,
        DisableMouseCapture
    );
    ratatui::restore();
}

impl App {
    /// The app over `stores`, each an account by its name; their folders
    /// are asked for at once. Its map is in memory alone.
    pub fn new(stores: Vec<(String, Box<dyn Store>)>, config: PathBuf) -> Self {
        Self::with_index(stores, config, Ok(Some(Index::memory())))
    }

    /// The app over `stores`, its map read from `index`; or not begun yet,
    /// none made; or with none, and why, when it could not be opened.
    pub fn with_index(
        stores: Vec<(String, Box<dyn Store>)>,
        config: PathBuf,
        index: Result<Option<Index>, String>,
    ) -> Self {
        let (index, map_off, map_later) = match index {
            Ok(Some(index)) => (index, None, false),
            Ok(None) => (Index::memory(), None, true),
            Err(why) => (Index::memory(), Some(why), false),
        };
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
            index: Arc::new(Mutex::new(index)),
            map_off,
            map_later,
            review: None,
            review_kept: Default::default(),
            twins: Vec::new(),
            open_map: open_map_index,
            syncs: Vec::new(),
            mapping: String::new(),
            map: Map::default(),
            map_corpus: None,
            attach_dir: None,
            reader_width: None,
            resizing: false,
            panes_right: 0,
            read_width: 0,
            look_file: None,
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
        let ident = store.ident();
        let twin = store.twin().filter(|_| self.map_off.is_none());
        let asks = worker::spawn(a, store, self.replies_to.clone());
        if self.map_later {
            self.twins.push(twin);
            self.syncs.push(None);
        } else {
            self.twins.push(None);
            self.syncs.push(
                twin.map(|twin| sync::spawn(a, twin, self.index.clone(), self.replies_to.clone())),
            );
        }
        self.accounts.push(Account {
            name,
            email: String::new(),
            ident,
            folders: Vec::new(),
            trash,
            state: State::Connecting,
            asks: Some(asks),
        });
        self.map_pass(a);
        a
    }

    /// Ask the account's sync to pass over it, for the map.
    fn map_pass(&mut self, account: usize) {
        let sent = self
            .syncs
            .get(account)
            .and_then(Option::as_ref)
            .is_some_and(|(s, _)| s.send(sync::Ask::Pass).is_ok());
        if sent {
            self.pending += 1;
        }
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
        self.map_due();
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
            self.map_due();
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
            Due::Folder => self.open_row(false),
            Due::Message if self.map.mode == Mode::Map => self.read_dot(),
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
        // How far the map's sync has got is said as it goes.
        if let Answer::Mapping(said) = reply.answer {
            self.mapping = said;
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
        if let Answer::Drawn(snapshot) = reply.answer {
            return self.drawn_map(*snapshot);
        }
        if let Answer::Relaid { lens, keys, at } = reply.answer {
            return self.relaid(lens, &keys, &at);
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
            | Answer::Mapping(_)
            | Answer::Drawn(_)
            | Answer::Relaid { .. }
            | Answer::Realm { .. } => {}
            Answer::Filed { key, label, filed } => self.filed(a, key, label, filed),
            Answer::LabelRenamed { old, result } => match result {
                Ok(()) => self.map_pass(a),
                Err(e) => self.status = format!("cannot rename {old}: {}", first_line(&e)),
            },
            Answer::Archived(result) => match result {
                Ok(()) => {
                    self.reload(a);
                    self.map_pass(a);
                }
                Err(e) => self.status = format!("cannot archive it: {}", first_line(&e)),
            },
            Answer::Mapped(passed) => {
                self.mapping = match passed {
                    Ok(_) => String::new(),
                    Err(e) => format!(
                        "the map cannot read {}: {}",
                        self.accounts[a].name,
                        first_line(&e)
                    ),
                };
            }
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
                    self.map_pass(a);
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
            Some(View::Map) => "Map, every cluster".to_string(),
            Some(View::Cluster(id)) => self
                .map
                .snapshot
                .as_ref()
                .and_then(|s| s.group(*id))
                .map(|g| g.name())
                .unwrap_or_else(|| "a cluster".to_string()),
            Some(View::Loners) => "Fits nowhere".to_string(),
            None => String::new(),
        }
    }

    /// The folder listed, as the sidebar has it, if one account's is.
    pub fn folder(&self) -> Option<&Folder> {
        match &self.listing.as_ref()?.view {
            View::Folder(a, path) => self.folder_of(*a, path),
            _ => None,
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
        }
        let fixed = self.listing.as_mut().and_then(|l| l.fixed.as_mut());
        std::iter::once(&mut self.shown)
            .chain(self.found.as_mut())
            .chain(fixed)
            .flatten()
            .filter(|m| m.account == account && m.folder == folder)
            .for_each(|m| set(&mut m.summary));
        if let Some((messages, _)) = self.listed.get_mut(&(account, folder.to_string())) {
            messages.iter_mut().for_each(&mut set);
        }
        self.index_flag(account, folder, uid, flag, on);
        // Flagged, it is kept: learnt as not thrown away.
        if changed && flag == Flag::Flagged && on {
            self.teach_at(account, folder, uid, false);
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
        if let Some(s) = &self.map.snapshot
            && !s.groups.is_empty()
        {
            self.rows.push(Row::MapHead);
            self.rows.push(Row::Map);
            self.rows
                .extend(s.groups.iter().map(|g| Row::Cluster(g.id)));
            if s.clustering.loner.iter().any(|&l| l) {
                self.rows.push(Row::Loners);
            }
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

    /// The message picked: on the map, where it is looked at, else in the
    /// list.
    fn picked(&self) -> Option<Listed> {
        if self.map.mode == Mode::Map && self.focus == Focus::List {
            return self.listed_dot(&self.dot()?);
        }
        self.current().cloned()
    }

    /// Archive the message picked: out of the Inbox, to Archive.
    fn archive(&mut self) {
        let Some(Listed {
            account,
            folder,
            summary,
        }) = self.picked()
        else {
            self.status = "no message to archive".to_string();
            return;
        };
        let archived = self.accounts[account]
            .folders
            .iter()
            .any(|f| f.path == folder && f.role == Role::Archive);
        if archived {
            self.status = "in Archive already".to_string();
            return;
        }
        self.drop_listed(account, &folder, summary.uid);
        self.status = "archived, out of the Inbox".to_string();
        self.ask(
            account,
            Ask::Archive {
                folder,
                uid: summary.uid,
                message_id: summary.message_id,
            },
        );
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
            Some(&Row::Map) => View::Map,
            Some(&Row::Cluster(id)) => View::Cluster(id),
            Some(&Row::Loners) => View::Loners,
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
            fixed: None,
        });
        self.query.clear();
        self.found = None;
        self.searching = false;
        // What was picked before is not picked again: the list starts at
        // its top, and the message read is let go, not left beside a list
        // it is not in.
        self.shown.clear();
        self.list.select(Some(0));
        *self.list.offset_mut() = 0;
        self.reading = None;
        self.scroll = 0;
        self.due = None;
        let folders = match view {
            View::Folder(a, path) => vec![(a, path)],
            View::All(role) => self.folders_of(role),
            View::Map | View::Cluster(_) | View::Loners => {
                self.list_map();
                Vec::new()
            }
        };
        for (a, path) in folders {
            self.list_part(a, path);
        }
        self.narrow();
    }

    /// Open what is picked in the sidebar. In the map, a cluster narrows
    /// it, and a folder is listed in place of it, if `go` is; else as
    /// `open_folder` does.
    fn open_row(&mut self, go: bool) {
        if self.map.mode == Mode::Map {
            match self.rows.get(self.row).copied() {
                Some(Row::Map) => return self.map.focused = Focused::All,
                Some(Row::Cluster(id)) => return self.map.focused = Focused::Cluster(id),
                Some(Row::Loners) => return self.map.focused = Focused::Loners,
                Some(Row::Folder(..) | Row::All(_)) if go => {
                    self.looked_now();
                    self.map.mode = Mode::List;
                }
                _ => return,
            }
        }
        self.open_folder();
    }

    /// What the map lists, if it is listed, as its snapshot now has it:
    /// every cluster, its unread first, each under its anchor; one; or what
    /// fits nowhere.
    fn list_map(&mut self) {
        let Some(view) = self.listing.as_ref().map(|l| l.view.clone()) else {
            return;
        };
        let Some(s) = self.map.snapshot.clone() else {
            if let Some(l) = &mut self.listing {
                l.fixed = Some(Vec::new());
            }
            return;
        };
        let members: Vec<usize> = match view {
            View::Map => s
                .groups
                .iter()
                .flat_map(|g| {
                    let mut m = g.members.clone();
                    m.sort_by_key(|&i| (s.dots[i].flags.seen, std::cmp::Reverse(s.dots[i].date)));
                    m
                })
                .collect(),
            View::Cluster(id) => (0..s.dots.len()).filter(|&i| s.picked_by(id, i)).collect(),
            View::Loners => s.loners(),
            _ => return,
        };
        let mut members = members;
        if !matches!(view, View::Map) {
            members.sort_by_key(|&i| std::cmp::Reverse(s.dots[i].date));
        }
        let listed: Vec<Listed> = members
            .into_iter()
            .filter_map(|i| self.listed_dot(&s.dots[i]))
            .collect();
        if let Some(l) = &mut self.listing {
            l.fixed = Some(listed);
        }
    }

    /// The account of the map's message, by its place, and the folder it is
    /// in: the Inbox, if it is there.
    fn place_of(&self, dot: &map::Dot) -> Option<(usize, String, u32)> {
        let a = self
            .live()
            .find(|&a| self.accounts[a].ident == dot.key.account)?;
        let place = dot
            .places
            .iter()
            .find(|p| self.inbox(a) == p.folder)
            .or_else(|| dot.places.first())?;
        Some((a, place.folder.clone(), place.uid))
    }

    /// The map's message as the list has it.
    fn listed_dot(&self, dot: &map::Dot) -> Option<Listed> {
        let (a, folder, uid) = self.place_of(dot)?;
        let summary = Summary {
            uid,
            from: dot.from.clone(),
            address: dot.address.clone(),
            to: String::new(),
            subject: dot.subject.clone(),
            date: dot.date,
            flags: dot.flags,
            preview: dot.preview.clone(),
            size: dot.size,
            message_id: dot.message_id.clone(),
        };
        Some(Listed::new(a, &folder, summary))
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
            self.map_pass(a);
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
            (
                _,
                Some(Listing {
                    fixed: Some(fixed), ..
                }),
            ) => fixed
                .iter()
                .filter(|m| {
                    words.is_empty() || {
                        let text = format!("{} {} {} {}", m.from, m.address, m.subject, m.preview)
                            .to_lowercase();
                        words.iter().all(|w| text.contains(w.as_str()))
                    }
                })
                .cloned()
                .collect(),
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
        let key = capital(key);
        if self.compose.is_some() {
            if matches!(self.dialog, Dialog::Attach(_) | Dialog::Help(_)) {
                return self.dialog_key(key);
            }
            // F1, as Emacs has it: every key, over the message.
            if key.code == KeyCode::F(1) {
                self.dialog = Dialog::Help(0);
                return;
            }
            return self.compose_key(key);
        }
        if key.modifiers.contains(KeyModifiers::CONTROL) && key.code == KeyCode::Char('c') {
            self.quit = true;
            return;
        }
        if self.dialog.is_open() {
            return self.dialog_key(key);
        }
        if self.review.as_ref().is_some_and(|r| r.menu.is_some()) {
            return self.menu_key(key);
        }
        if let Some(r) = self.review.as_mut().filter(|r| r.asking) {
            r.asking = false;
            if matches!(key.code, KeyCode::Char('y' | 'Y')) && key.modifiers.is_empty() {
                self.apply_review();
            } else {
                self.status = "nothing done · $ when it is as you want it".to_string();
            }
            return;
        }
        if self.searching {
            match key.code {
                KeyCode::Esc => {
                    self.query.clear();
                    self.found = None;
                    self.map.hits = None;
                    self.searching = false;
                }
                // On the map, what is found is lit there; nothing is asked of
                // the servers.
                KeyCode::Enter if self.map.mode == Mode::Map => {
                    self.searching = false;
                    let now = jiff::Timestamp::now().as_second();
                    let found = self.map.seek(&self.query, now);
                    self.status =
                        format!("{found} found · n next, p the one before, esc lets it go");
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
            if self.map.mode == Mode::Map {
                self.map
                    .seek(&self.query, jiff::Timestamp::now().as_second());
            }
            return;
        }
        if let Some(act) = keymap::act(key) {
            self.act(act);
        }
    }

    /// Do what a key in the table does, where the focus is.
    fn act(&mut self, act: Act) {
        if self.review_act(act) {
            return;
        }
        if self.map.mode == Mode::Map && self.focus == Focus::List && self.map_act(act) {
            return;
        }
        let page = self.height.saturating_sub(2).max(1) as isize;
        match act {
            Act::Quit => self.quit = true,
            Act::Help => self.dialog = Dialog::Help(0),
            Act::Review => self.open_review(),
            Act::Apply => {
                self.status =
                    "$ does what was chosen in the Inbox looked through: i opens it".to_string()
            }
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
            // The map has no sidebar: between it and the message read.
            Act::NextPane | Act::PreviousPane if self.map.mode == Mode::Map => {
                self.focus = if self.focus == Focus::List
                    && (self.reading.is_some() || self.review.is_some())
                {
                    Focus::Reader
                } else {
                    Focus::List
                };
            }
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
                    self.open_row(true);
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
                    self.open_row(true);
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
            Act::Lens
            | Act::Since
            | Act::Colours
            | Act::NewOnly
            | Act::ShowRead
            | Act::ShowSent
            | Act::Unfiled => self.view(act),
            Act::View if self.map.mode == Mode::Map => self.dialog = Dialog::View,
            Act::Mark => self.mark_cluster(),
            Act::Move
            | Act::NewLabel
            | Act::FileFirst
            | Act::FileAll
            | Act::NotIt
            | Act::View
            | Act::ZoomIn
            | Act::ZoomOut
            | Act::ZoomAll
            | Act::Relayout => {
                self.status = "M-g shows the map, where mail is filed".to_string();
            }
            Act::Wider | Act::Narrower if self.read_width > 0 => {
                let by: i16 = if act == Act::Wider { 4 } else { -4 };
                self.reader_width = Some(self.read_width.saturating_add_signed(by));
            }
            Act::Wider | Act::Narrower => {}
            Act::MapView => self.show_map(),
            Act::ListView => self.show_list(),
            Act::PreviousCluster => self.map.cycle(-1),
            Act::NextCluster => self.map.cycle(1),
            Act::Files => self.list_files(),
            Act::Archive => self.archive(),
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
            Dialog::Attach(_) => self.attach_key(key),
            Dialog::View => self.view_key(key),
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
                self.open_item();
            }
            (Dialog::Files(_), _) => self.dialog = Dialog::Closed,
            (Dialog::Attach(picker), Some(Target::Item(i))) => picker.click(i),
            (Dialog::Attach(_), _) => self.dialog = Dialog::Closed,
            (Dialog::View, Some(Target::Item(i))) => {
                if let Some(&(_, act, _)) = keymap::VIEW.get(i) {
                    self.view(act);
                }
            }
            (Dialog::View, _) => self.dialog = Dialog::Closed,
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
                    if matches!(
                        self.rows[at],
                        Row::All(_)
                            | Row::Folder(..)
                            | Row::Add
                            | Row::Map
                            | Row::Cluster(_)
                            | Row::Loners
                    ) {
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
        let at = self
            .targets
            .iter()
            .rev()
            .find(|(r, _)| r.contains(Position::new(m.column, m.row)))
            .map(|&(_, t)| t);
        // Writing, its keys are clicked, and nothing behind it; the files
        // being picked, or the help, as they are elsewhere.
        if self.compose.is_some() && !matches!(self.dialog, Dialog::Attach(_) | Dialog::Help(_)) {
            if m.kind == MouseEventKind::Down(MouseButton::Left)
                && let Some(key @ (Target::Key(_) | Target::Chord(..))) = at
            {
                self.click(Some(key));
            }
            return;
        }
        let by = match m.kind {
            MouseEventKind::Down(MouseButton::Left) => {
                if at == Some(Target::Divider) && !self.dialog.is_open() {
                    self.resizing = true;
                    return;
                }
                // The labels dropped down: a click elsewhere puts them away.
                if let Some(r) = self.review.as_mut().filter(|r| r.menu.is_some())
                    && !matches!(at, Some(Target::MenuItem(_)))
                {
                    r.menu = None;
                    return;
                }
                if at == Some(Target::Map) && !self.dialog.is_open() {
                    return self.map_press(m.column, m.row);
                }
                return self.click(at);
            }
            MouseEventKind::Drag(MouseButton::Left) if self.resizing => {
                self.reader_width = Some(self.panes_right.saturating_sub(m.column));
                return;
            }
            MouseEventKind::Up(MouseButton::Left) if self.resizing => {
                self.resizing = false;
                return;
            }
            MouseEventKind::Drag(MouseButton::Left) => return self.map_drag(m.column, m.row),
            MouseEventKind::Up(MouseButton::Left) => return self.map_release(m.column, m.row),
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
            // The wheel on the Inbox looked through goes along it.
            Some(Target::ReviewRow(_) | Target::Choose(..)) => self.review_step(by),
            // The wheel on a cluster's tree goes along it.
            Some(Target::Map | Target::Branch(_)) if self.map.tree.is_some() => {
                self.map_step(0, by.signum() as i32)
            }
            // The wheel on the map zooms, about where it is.
            Some(Target::Map | Target::Anchor(_)) => {
                let factor = if by < 0 {
                    map::ZOOM_STEP
                } else {
                    1.0 / map::ZOOM_STEP
                };
                self.map.zoom_at(factor, m.column, m.row);
            }
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
            Some(Target::Chord(a, b)) => {
                self.key(a);
                self.key(b);
            }
            Some(Target::Search) => self.searching = true,
            Some(Target::Side) => self.focus = Focus::Side,
            Some(Target::Row(i)) if self.rows.get(i) == Some(&Row::Add) => {
                self.row = i;
                self.open_form();
            }
            Some(Target::Row(i)) => {
                self.focus = Focus::Side;
                if matches!(
                    self.rows.get(i),
                    Some(Row::Folder(..) | Row::All(_) | Row::Map | Row::Cluster(_) | Row::Loners)
                ) {
                    self.row = i;
                    self.open_row(true);
                    self.focus = Focus::List;
                }
            }
            Some(Target::List) => self.focus = Focus::List,
            Some(Target::Anchor(id)) => self.anchor_click(id),
            Some(Target::Branch(row)) => self.branch_click(row),
            Some(Target::ReviewRow(row)) => self.review_click(row),
            Some(Target::Choose(row, which)) => self.choice_click(row, which),
            Some(Target::MenuItem(at)) => self.pick_item(at),
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
        let mut items: Vec<Item> = vec![Item::Page];
        items.extend(m.attachments.iter().enumerate().map(|(at, a)| Item::File {
            at,
            name: a.name.clone(),
            mime: a.mime.clone(),
            size: a.size,
        }));
        items.extend(m.links.iter().cloned().map(Item::Link));
        // On its first file, if it has one; else on the message itself.
        let at = usize::from(items.len() > 1);
        self.dialog = Dialog::Files(Files {
            items,
            at,
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
            (_, KeyCode::Char('S')) => {
                self.dialog = Dialog::Closed;
                self.save_attachments();
            }
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
            Some(Item::Page) => {
                self.dialog = Dialog::Closed;
                self.open_in_browser();
            }
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

    /// Learn from what was done with the message of `account` at `uid` in
    /// `folder`: thrown away, or kept.
    fn teach_at(&mut self, account: usize, folder: &str, uid: u32, thrown: bool) {
        let Some(ident) = self.accounts.get(account).map(|a| a.ident.clone()) else {
            return;
        };
        let mut index = self.index.lock().unwrap_or_else(|e| e.into_inner());
        if let Some(key) = index.key_at(&ident, folder, uid) {
            let _ = index.teach(&key, thrown);
        }
    }

    /// Learn from what was done with the messages `keys`: thrown away, or
    /// kept.
    fn teach(&mut self, keys: &[crate::index::Key], thrown: bool) {
        let mut index = self.index.lock().unwrap_or_else(|e| e.into_inner());
        for key in keys {
            let _ = index.teach(key, thrown);
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
        // Thrown away: what is learnt of what you throw away.
        self.teach_at(account, &from, uid, true);
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
        let Some(summary) = self.picked() else {
            self.status = "no message to answer".to_string();
            return;
        };
        let (account, folder, uid) = (summary.account, summary.folder, summary.summary.uid);
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
    fn attach_key(&mut self, key: KeyEvent) {
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
    fn attach(&mut self, paths: &[PathBuf]) {
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
    /// is a file; else written in, or typed into the search.
    pub fn paste(&mut self, text: &str) {
        if self.compose.is_some() && !self.dialog.is_open() {
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

// ---- the map ----

impl App {
    /// Do what a key does in the map, with the focus on it; whether it
    /// was the map's to do.
    fn map_act(&mut self, act: Act) -> bool {
        if self.map.preview.is_some() {
            match act {
                Act::Open => {
                    self.keep_relayout();
                    return true;
                }
                Act::Back => {
                    self.map.preview = None;
                    self.status = "the map as it was".to_string();
                    return true;
                }
                _ => {}
            }
        }
        if act == Act::Relayout {
            self.relayout();
            return true;
        }
        if self.map.moving.is_some() {
            match act {
                Act::Open => {
                    let id = self
                        .map
                        .picked()
                        .and_then(|i| self.map.snapshot.as_ref()?.group_of(i).map(|g| g.id));
                    if let Some(id) = id {
                        self.move_onto(id);
                    }
                    return true;
                }
                Act::NewLabel => {
                    self.ask_new_label();
                    return true;
                }
                Act::Back => {
                    self.map.moving = None;
                    self.status = "not moved".to_string();
                    // Picking where a message looked through goes: back to it.
                    if let Some(r) = &mut self.review
                        && r.relabel.take().is_some()
                    {
                        self.focus = Focus::Reader;
                    }
                    return true;
                }
                _ => {}
            }
        }
        match act {
            Act::Move => {
                self.start_moving();
                return true;
            }
            Act::FileFirst => {
                self.file_suggested(false);
                return true;
            }
            Act::FileAll => {
                self.file_suggested(true);
                return true;
            }
            Act::NotIt => {
                self.not_it();
                return true;
            }
            Act::Rename => {
                self.ask_rename_label();
                return true;
            }
            Act::Back if !self.query.is_empty() && self.map.region.is_none() => {
                self.query.clear();
                self.found = None;
                self.map.hits = None;
                self.narrow();
                self.status = "the search let go".to_string();
                return true;
            }
            Act::ZoomIn if self.map.tree.is_some() => {
                self.map.tree_in(jiff::Timestamp::now().as_second());
                return true;
            }
            Act::ZoomIn => {
                self.map.zoom_picked(map::ZOOM_STEP);
                return true;
            }
            Act::ZoomOut => {
                self.map.zoom_picked(1.0 / map::ZOOM_STEP);
                return true;
            }
            Act::ZoomAll => {
                self.map.zoom_all();
                return true;
            }
            // From farther than all of it, a cluster's node is opened.
            Act::Open if self.map.clusters => {
                let id = self
                    .map
                    .picked()
                    .and_then(|i| self.map.snapshot.as_ref()?.group_of(i).map(|g| g.id));
                if let Some(id) = id {
                    self.map.open_node(id);
                }
                return true;
            }
            // In a cluster's tree, ⏎ goes into the branch picked, or, a
            // message, reads it.
            Act::Open if self.map.tree.is_some() => {
                if !self.map.tree_in(jiff::Timestamp::now().as_second()) {
                    self.read_dot();
                }
                return true;
            }
            Act::NextMessage | Act::PreviousMessage if !self.query.is_empty() => {
                let by = if act == Act::NextMessage { 1 } else { -1 };
                let now = jiff::Timestamp::now().as_second();
                self.map.next_found(&self.query.clone(), by, now);
                if self.side_by_side {
                    self.due = Some((Instant::now(), Due::Message));
                }
                return true;
            }
            Act::Back if self.map.suggesting.is_some() && self.map.region.is_none() => {
                self.map.suggesting = None;
                return true;
            }
            Act::Back if self.map.tree.is_some() && self.map.region.is_none() => {
                self.map.close_tree();
                return true;
            }
            _ => {}
        }
        if self.map.region.is_some() {
            match act {
                Act::Seen => return self.region_flag(Flag::Seen),
                Act::Flag => return self.region_flag(Flag::Flagged),
                Act::Trash => return self.region_trash(),
                Act::Archive => return self.region_archive(),
                Act::Back => {
                    self.map.region = None;
                    self.status = "let go".to_string();
                    return true;
                }
                _ => {}
            }
        }
        match act {
            Act::Down => self.map_step(0, 1),
            Act::Up => self.map_step(0, -1),
            Act::Left => self.map_step(-1, 0),
            Act::Right => self.map_step(1, 0),
            Act::Open => self.read_dot(),
            Act::Seen => self.dot_flag(Flag::Seen),
            Act::Flag => self.dot_flag(Flag::Flagged),
            Act::Trash => self.delete_dot(),
            Act::Archive => {
                if let Some(dot) = self.dot() {
                    self.mark(vec![dot.key], None);
                    self.region_archive();
                }
            }
            Act::Back if self.map.focused != Focused::All => self.map.focused = Focused::All,
            Act::Back
            | Act::Top
            | Act::End
            | Act::PageDown
            | Act::PageUp
            | Act::ScreenDown
            | Act::ScreenUp
            | Act::NextMessage
            | Act::PreviousMessage => {}
            _ => return false,
        }
        true
    }

    /// A key in the map's view menu: what it shows changed, the menu kept
    /// open to change more; or the map laid out again; any other key
    /// closes it.
    fn view_key(&mut self, key: KeyEvent) {
        let row = match key.code {
            KeyCode::Char(c) => keymap::VIEW.iter().find(|v| v.0 == c),
            _ => None,
        };
        match row {
            Some(&(_, act, _)) => self.view(act),
            None => self.dialog = Dialog::Closed,
        }
    }

    /// Change what the map shows, or how, as `act` says.
    fn view(&mut self, act: Act) {
        self.status = match act {
            Act::Lens => {
                self.map.lens = match self.map.lens {
                    crate::index::Lens::Both => crate::index::Lens::Words,
                    crate::index::Lens::Words => crate::index::Lens::People,
                    crate::index::Lens::People => crate::index::Lens::Both,
                };
                format!("the map laid out by {}", self.map.lens.name())
            }
            Act::Since => {
                self.map.since = self.map.since.next();
                format!("the map of {}", self.map.since.name())
            }
            Act::Colours => {
                self.map.colour = self.map.colour.next();
                self.map.colour.explain().to_string()
            }
            Act::ShowRead => {
                self.map.hide_read = !self.map.hide_read;
                if self.map.hide_read {
                    "read mail hidden"
                } else {
                    "read mail shown"
                }
                .to_string()
            }
            Act::ShowSent => {
                self.map.hide_sent = !self.map.hide_sent;
                if self.map.hide_sent {
                    "sent mail hidden"
                } else {
                    "sent mail shown"
                }
                .to_string()
            }
            Act::Unfiled => {
                self.map.unfiled_only = !self.map.unfiled_only;
                if self.map.unfiled_only {
                    "only the mail not yet filed under a label"
                } else {
                    "filed mail shown too"
                }
                .to_string()
            }
            Act::NewOnly => {
                self.map.new_only = !self.map.new_only;
                if self.map.new_only {
                    "areas with nothing new dimmed"
                } else {
                    "every area as bright as it is"
                }
                .to_string()
            }
            Act::Relayout => {
                self.dialog = Dialog::Closed;
                return self.relayout();
            }
            _ => return,
        };
    }

    /// Move the map's cursor that way, and read what it picks once moving
    /// pauses, where the message read is beside the map.
    fn map_step(&mut self, dx: i32, dy: i32) {
        let now = jiff::Timestamp::now().as_second();
        if self.map.tree.is_some() {
            match dx.signum() {
                1 => {
                    self.map.tree_in(now);
                }
                -1 => self.map.tree_out(),
                _ => self.map.tree_along(dy as isize, now),
            }
        } else if self.map.clusters {
            self.map.step_node(dx, dy);
        } else {
            self.map.step(dx, dy);
        }
        if self.side_by_side {
            self.due = Some((Instant::now(), Due::Message));
        }
    }

    /// Begin the map, as was agreed: its key made and its index opened, and
    /// each account's sync set going; then show it. What stops it is said.
    fn start_map(&mut self) {
        self.dialog = Dialog::Closed;
        self.map_later = false;
        let index = match (self.open_map)() {
            Ok(index) => index,
            Err(why) => {
                self.status = format!("no map: {why}");
                self.map_off = Some(why);
                return;
            }
        };
        *self.index.lock().unwrap_or_else(|e| e.into_inner()) = index;
        for a in 0..self.twins.len() {
            if let Some(twin) = self.twins[a].take() {
                self.syncs[a] = Some(sync::spawn(
                    a,
                    twin,
                    self.index.clone(),
                    self.replies_to.clone(),
                ));
                self.map_pass(a);
            }
        }
        self.show_map();
    }

    /// Open as the mail was last looked at: the map, from as far as its
    /// clusters if they were shown, else the list. Never looked at, the
    /// list, and the map, when it is, from its clusters.
    pub fn recall_look(&mut self) {
        let Some(said) = self
            .look_file
            .as_ref()
            .and_then(|p| std::fs::read_to_string(p).ok())
        else {
            self.map.clusters = true;
            return;
        };
        let mut words = said.split_whitespace();
        if words.next() != Some("map") || self.map_off.is_some() || self.map_later {
            return;
        }
        self.show_map();
        self.map.clusters = words.next() == Some("clusters");
    }

    /// Write down how the mail is looked at, to open so again.
    pub fn remember_look(&self) {
        let Some(path) = &self.look_file else {
            return;
        };
        let look = match self.map.mode {
            Mode::Map if self.map.clusters || self.map.tree.is_some() => "map clusters",
            Mode::Map => "map",
            Mode::List => "list",
        };
        // Only how it was left; nothing lost if it cannot be kept.
        let _ = std::fs::write(path, format!("{look}\n"));
    }

    /// Show the map in the middle pane, mail new since it was last looked
    /// at marked.
    fn show_map(&mut self) {
        if let Some(why) = &self.map_off {
            self.status = format!("no map: {why}");
            return;
        }
        if self.map_later {
            self.dialog = Dialog::StartMap;
            return;
        }
        if self.map.mode != Mode::Map {
            let looked = self
                .index
                .lock()
                .unwrap_or_else(|e| e.into_inner())
                .keep
                .looked;
            self.map.looked = (looked != 0).then_some(looked);
        }
        self.map.mode = Mode::Map;
        self.searching = false;
        self.focus = Focus::List;
        self.map.pick_first(jiff::Timestamp::now().as_second());
        self.map_due();
    }

    /// Show the list again; the map was looked at until now.
    fn show_list(&mut self) {
        if self.map.mode == Mode::Map {
            self.looked_now();
            // What the map was narrowed to is what is listed.
            let narrowed = match self.map.focused {
                Focused::Cluster(id) => Some(Row::Cluster(id)),
                Focused::Loners => Some(Row::Loners),
                Focused::All => None,
            };
            if let Some(i) = narrowed.and_then(|n| self.rows.iter().position(|r| *r == n)) {
                self.row = i;
            }
        }
        self.map.mode = Mode::List;
        if self.focus == Focus::Reader && !self.side_by_side {
            self.focus = Focus::List;
        }
        if matches!(
            self.rows.get(self.row),
            Some(Row::Map | Row::Cluster(_) | Row::Loners)
        ) {
            self.open_folder();
        }
    }

    /// Write down that the map was looked at now: what comes after is new.
    fn looked_now(&mut self) {
        let at = jiff::Timestamp::now().as_second();
        let kept = self
            .index
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .keep(vec![crate::index::Kept::Looked { at }]);
        if let Err(e) = kept {
            self.status = format!("the map cannot keep when it was looked at: {e:#}");
        }
        self.map.looked = None;
    }

    /// The message picked on the map.
    fn dot(&self) -> Option<map::Dot> {
        let s = self.map.snapshot.as_ref()?;
        Some(s.dots[self.map.picked()?].clone())
    }

    /// Read the message picked on the map.
    fn read_dot(&mut self) {
        let Some(dot) = self.dot() else {
            self.status = "no message picked: the arrows or hjkl pick one".to_string();
            return;
        };
        let Some((a, folder, uid)) = self.place_of(&dot) else {
            self.status = "that message is on no account here".to_string();
            return;
        };
        self.due = None;
        self.scroll = 0;
        let mark = self.mark_read && !dot.flags.seen;
        let body = match self.read.get(&(a, folder.clone(), uid)) {
            Some(m) => {
                if mark {
                    self.marked(a, &folder, uid, Flag::Seen, true);
                    self.ask(
                        a,
                        Ask::Set {
                            folder: folder.clone(),
                            uid,
                            flag: Flag::Seen,
                            on: true,
                        },
                    );
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
        if !self.side_by_side {
            self.focus = Focus::Reader;
        }
    }

    /// Mark the message picked on the map, or not, as `flag` says.
    fn dot_flag(&mut self, flag: Flag) {
        let Some(dot) = self.dot() else {
            return;
        };
        let Some((a, folder, uid)) = self.place_of(&dot) else {
            return;
        };
        let on = match flag {
            Flag::Seen => !dot.flags.seen,
            Flag::Flagged => !dot.flags.flagged,
            Flag::Answered => !dot.flags.answered,
        };
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

    /// Move the message picked on the map to Trash.
    fn delete_dot(&mut self) {
        let Some(dot) = self.dot() else {
            return;
        };
        let Some(listed) = self.listed_dot(&dot) else {
            return;
        };
        let (a, folder, summary) = (listed.account, listed.folder, listed.summary);
        match self.trash_of(a) {
            Some(trash) => self.move_to_trash(a, folder, summary.uid, summary.message_id, trash),
            None => self.ask_which_trash(a, folder, summary),
        }
    }

    /// Show a message marked, or not, on the map too: in the index, as its
    /// server will say at the next pass.
    fn index_flag(&mut self, account: usize, folder: &str, uid: u32, flag: Flag, on: bool) {
        let Some(ident) = self.accounts.get(account).map(|a| a.ident.clone()) else {
            return;
        };
        let mut index = self.index.lock().unwrap_or_else(|e| e.into_inner());
        let found = index
            .records
            .values()
            .find(|r| r.key.account == ident && r.place(folder).is_some_and(|p| p.uid == uid))
            .map(|r| (r.key.clone(), r.flags));
        if let Some((key, mut flags)) = found {
            match flag {
                Flag::Seen => flags.seen = on,
                Flag::Flagged => flags.flagged = on,
                Flag::Answered => flags.answered = on,
            }
            let _ = index.cache(vec![crate::index::Cached::Flags { key, flags }]);
        }
    }

    /// Make the map's snapshot again, off this thread, if the index has
    /// changed since, or the map is to be laid out and never was.
    fn map_due(&mut self) {
        if self.map.computing || self.map_off.is_some() || self.map_later {
            return;
        }
        let layout = self.map.mode == Mode::Map;
        let shared = self.index.clone();
        let index = shared.lock().unwrap_or_else(|e| e.into_inner());
        let stale = match &self.map.snapshot {
            None => layout || !index.records.is_empty(),
            Some(s) => {
                s.version != index.version || s.lens != self.map.lens || (layout && s.at.is_none())
            }
        };
        if !stale {
            return;
        }
        let input = map::input(
            &index,
            self.map.lens,
            self.map.snapshot.as_deref(),
            self.map_corpus.take(),
            layout,
        );
        drop(index);
        self.map.computing = true;
        self.pending += 1;
        let replies = self.replies_to.clone();
        std::thread::spawn(move || {
            let snapshot = map::compute(input);
            let _ = replies.send(Reply {
                account: usize::MAX,
                answer: Answer::Drawn(Box::new(snapshot)),
            });
        });
    }

    /// Take the map's new snapshot: keep a layout made for it, keep the
    /// message picked if it is still there, and list again what the map
    /// lists.
    fn drawn_map(&mut self, mut snapshot: map::Snapshot) {
        self.map.computing = false;
        self.map_corpus = snapshot.corpus.take();
        if let Some(fresh) = &snapshot.fresh {
            let kept = self
                .index
                .lock()
                .unwrap_or_else(|e| e.into_inner())
                .keep(vec![crate::index::Kept::Layout {
                    lens: snapshot.lens,
                    at: fresh.clone(),
                }]);
            if let Err(e) = kept {
                self.status = format!("the map cannot keep its layout: {e:#}");
            }
        }
        if self
            .map
            .cursor
            .as_ref()
            .is_some_and(|k| snapshot.find(k).is_none())
        {
            self.map.cursor = None;
        }
        if let Focused::Cluster(id) = self.map.focused
            && snapshot.group(id).is_none()
        {
            self.map.focused = Focused::All;
        }
        // A layout shown to be kept or dropped follows the map, by key.
        if let (Some(p), Some(old)) = (self.map.preview.take(), self.map.snapshot.clone()) {
            self.map.preview = (old.lens == snapshot.lens)
                .then(|| map::onto(&snapshot, &old.model.keys, &p))
                .flatten();
        }
        self.map.snapshot = Some(Arc::new(snapshot));
        if self.map.mode == Mode::Map {
            self.map.pick_first(jiff::Timestamp::now().as_second());
        }
        self.rebuild();
        if self
            .listing
            .as_ref()
            .is_some_and(|l| matches!(l.view, View::Map | View::Cluster(_) | View::Loners))
        {
            self.list_map();
            self.narrow();
        }
        self.map_due();
    }

    /// Pick what is drawn in the map's cell at `column`, `row`: its
    /// message, or, clicked again, the next of those in it.
    fn map_click(&mut self, column: u16, row: u16) {
        self.focus = Focus::List;
        let Some(members) = self.map.drawn.get(&(column, row)).cloned() else {
            return;
        };
        let Some(s) = self.map.snapshot.clone() else {
            return;
        };
        let next = match self.map.clicked {
            Some((cell, at)) if cell == (column, row) => (at + 1) % members.len(),
            _ => 0,
        };
        self.map.clicked = Some(((column, row), next));
        self.map.cursor = Some(s.dots[members[next]].key.clone());
        if self.side_by_side {
            self.read_dot();
        }
    }

    /// Mark the cluster of the message picked on the map; or, marked, let
    /// what is marked go.
    fn mark_cluster(&mut self) {
        if self.map.mode != Mode::Map {
            return;
        }
        if self.map.region.take().is_some() {
            self.status = "let go".to_string();
            return;
        }
        let Some(s) = self.map.snapshot.clone() else {
            return;
        };
        let Some(g) = self.map.picked().and_then(|i| s.group_of(i)) else {
            self.status = "no message picked: the arrows or hjkl pick one".to_string();
            return;
        };
        let keys = g.members.iter().map(|&i| s.dots[i].key.clone()).collect();
        self.mark(keys, None);
    }

    /// Mark the messages `keys`, in the box `boxed` if they were drawn
    /// round, and say what is done with them.
    fn mark(&mut self, keys: Vec<crate::index::Key>, boxed: Option<((u16, u16), (u16, u16))>) {
        if keys.is_empty() {
            self.map.region = None;
            self.status = "nothing marked: the box holds no message".to_string();
            return;
        }
        self.status = format!(
            "{} marked: d trashes them, a archives them, u and s mark them, v lets them go",
            keys.len()
        );
        self.map.region = Some(map::Region { keys, boxed });
    }

    /// The messages marked on the map.
    fn region_dots(&self) -> Vec<map::Dot> {
        let (Some(s), Some(r)) = (&self.map.snapshot, &self.map.region) else {
            return Vec::new();
        };
        r.keys
            .iter()
            .filter_map(|k| s.find(k).map(|i| s.dots[i].clone()))
            .collect()
    }

    /// Mark every message marked on the map as `flag` says: all of them,
    /// unless all of them are, and then none.
    fn region_flag(&mut self, flag: Flag) -> bool {
        let dots = self.region_dots();
        let has = |d: &map::Dot| match flag {
            Flag::Seen => d.flags.seen,
            Flag::Flagged => d.flags.flagged,
            Flag::Answered => d.flags.answered,
        };
        let on = !dots.iter().all(has);
        for d in &dots {
            if has(d) == on {
                continue;
            }
            let Some((a, folder, uid)) = self.place_of(d) else {
                continue;
            };
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
        let what = match (flag, on) {
            (Flag::Seen, true) => "read",
            (Flag::Seen, false) => "unread",
            (Flag::Flagged, true) => "flagged",
            (Flag::Flagged, false) => "unflagged",
            (Flag::Answered, _) => "answered",
        };
        self.status = format!("{} marked {what}", dots.len());
        true
    }

    /// Move every message marked on the map to its account's Trash.
    fn region_trash(&mut self) -> bool {
        let dots = self.region_dots();
        let mut moved = 0;
        for d in &dots {
            let Some(listed) = self.listed_dot(d) else {
                continue;
            };
            let Some(trash) = self.trash_of(listed.account) else {
                continue;
            };
            let s = listed.summary;
            self.move_to_trash(listed.account, listed.folder, s.uid, s.message_id, trash);
            moved += 1;
        }
        self.map.region = None;
        self.status = if moved < dots.len() {
            format!(
                "{moved} of {} moved to Trash: d on one alone says which folder is Trash",
                dots.len()
            )
        } else {
            format!("{moved} moved to Trash · d there puts one back, for 7 days")
        };
        true
    }

    /// Take every message marked on the map that is in the Inbox out of it,
    /// given no label.
    fn region_archive(&mut self) -> bool {
        let dots = self.region_dots();
        let mut archived = 0;
        for d in dots.iter().filter(|d| d.system.inbox) {
            let Some((a, folder, uid)) = self.place_of(d) else {
                continue;
            };
            self.ask(
                a,
                Ask::Archive {
                    folder,
                    uid,
                    message_id: d.message_id.clone(),
                },
            );
            archived += 1;
        }
        self.map.region = None;
        self.status = format!("{archived} archived, out of the Inbox");
        true
    }

    /// The mouse pressed on the map, at `column`, `row`: a box begun.
    fn map_press(&mut self, column: u16, row: u16) {
        self.map.drag = Some(((column, row), (column, row)));
    }

    /// The mouse moved, pressed, to `column`, `row`: the box drawn so far.
    fn map_drag(&mut self, column: u16, row: u16) {
        if let Some((start, _)) = self.map.drag {
            self.map.drag = Some((start, (column, row)));
        }
    }

    /// The mouse let go at `column`, `row`: where it was pressed, a click;
    /// else what is in the box drawn, marked.
    fn map_release(&mut self, column: u16, row: u16) {
        let Some((start, _)) = self.map.drag.take() else {
            return;
        };
        if start == (column, row) {
            return self.map_click(column, row);
        }
        let (x0, x1) = (start.0.min(column), start.0.max(column));
        let (y0, y1) = (start.1.min(row), start.1.max(row));
        let Some(s) = self.map.snapshot.clone() else {
            return;
        };
        let keys = self
            .map
            .cells
            .iter()
            .filter(|&(_, &(x, y))| (x0..=x1).contains(&x) && (y0..=y1).contains(&y))
            .map(|(&i, _)| s.dots[i].key.clone())
            .collect();
        self.focus = Focus::List;
        self.mark(keys, Some(((x0, y0), (x1, y1))));
    }

    /// Begin moving the message picked, or what is marked, onto a cluster
    /// or a label.
    fn start_moving(&mut self) {
        let keys = match &self.map.region {
            Some(r) => r.keys.clone(),
            None => match self.dot() {
                Some(d) => vec![d.key],
                None => {
                    self.status = "no message picked: the arrows or hjkl pick one".to_string();
                    return;
                }
            },
        };
        self.status = format!(
            "moving {}: ⏎ on a cluster, or a click on its anchor; N a new label; esc stops",
            if keys.len() == 1 {
                "it".to_string()
            } else {
                keys.len().to_string()
            }
        );
        self.map.moving = Some(keys);
    }

    /// Move what is moving onto the cluster `id`: under its label, or, if it
    /// is none yet, under one asked for, its first word given.
    fn move_onto(&mut self, id: u32) {
        let Some(g) = self
            .map
            .snapshot
            .as_ref()
            .and_then(|s| s.group(id))
            .cloned()
        else {
            return;
        };
        match g.label {
            Some(label) => {
                let keys = self.map.moving.take().unwrap_or_default();
                self.file_keys(keys, &label);
            }
            None => {
                let word = g.words.first().cloned().unwrap_or_default();
                self.dialog = Dialog::Label(capitalised(&word));
            }
        }
    }

    /// Ask the name of a new label for what is moving, its commonest word
    /// given.
    fn ask_new_label(&mut self) {
        let dots: Vec<map::Dot> = {
            let (Some(s), Some(keys)) = (&self.map.snapshot, &self.map.moving) else {
                return;
            };
            keys.iter()
                .filter_map(|k| s.find(k).map(|i| s.dots[i].clone()))
                .collect()
        };
        let mut counts: std::collections::BTreeMap<String, usize> = Default::default();
        for d in &dots {
            for w in crate::model::words(crate::model::bare_subject(&d.subject)) {
                *counts.entry(w).or_default() += 1;
            }
        }
        let word = counts
            .into_iter()
            .max_by(|a, b| a.1.cmp(&b.1).then(b.0.cmp(&a.0)))
            .map(|(w, _)| w)
            .unwrap_or_default();
        self.dialog = Dialog::Label(capitalised(&word));
    }

    /// File the messages `keys` under `label`: shown at once as given, and
    /// each asked of its server; the label's suggestions shown after.
    fn file_keys(&mut self, keys: Vec<crate::index::Key>, label: &str) {
        let label = label.trim().to_string();
        self.map.moving = None;
        // Picked for a message looked through: chosen, to be done with the
        // rest.
        if let Some(r) = &mut self.review
            && let Some(k) = r.relabel.take()
            && keys == [k.clone()]
            && !label.is_empty()
        {
            r.chosen.insert(k, Choice::Move(label.clone()));
            self.status = format!("to go to {label} · $ does it, with the rest");
            self.focus = Focus::Reader;
            return;
        }
        if label.is_empty() || keys.is_empty() {
            self.status = "not filed".to_string();
            return;
        }
        let Some(s) = self.map.snapshot.clone() else {
            return;
        };
        // Filed, it is kept: learnt as not thrown away.
        self.teach(&keys, false);
        let at = jiff::Timestamp::now().as_second();
        let mut filed = 0;
        let mut pending = Vec::new();
        let mut asks = Vec::new();
        for key in &keys {
            let Some(i) = s.find(key) else { continue };
            let dot = &s.dots[i];
            let Some((a, folder, uid)) = self.place_of(dot) else {
                continue;
            };
            pending.push(crate::index::Kept::Pending {
                key: key.clone(),
                label: label.clone(),
                on: true,
                at,
            });
            asks.push((
                a,
                Ask::File {
                    folder,
                    uid,
                    label: label.clone(),
                    message_id: dot.message_id.clone(),
                    key: key.clone(),
                },
            ));
            filed += 1;
        }
        let kept = self
            .index
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .keep(pending);
        if let Err(e) = kept {
            self.status = format!("cannot file it: {e:#}");
            return;
        }
        for (a, ask) in asks {
            self.ask(a, ask);
        }
        self.map.region = None;
        self.map.suggesting = Some(label.clone());
        self.status = format!(
            "{} under {label}, out of the Inbox · y files the next it would go with too",
            if filed == 1 {
                "filed".to_string()
            } else {
                format!("{filed} filed")
            }
        );
    }

    /// The suggestions for the label last filed under, nearest first.
    pub fn suggestions(&self) -> Vec<(usize, f32)> {
        let (Some(s), Some(label)) = (&self.map.snapshot, &self.map.suggesting) else {
            return Vec::new();
        };
        s.model.suggestions(label, &s.not_it)
    }

    /// The message picked, if it is filed under no label yet, and the label
    /// it would go to.
    fn picked_suggestion(&self) -> Option<(usize, String)> {
        let s = self.map.snapshot.as_ref()?;
        let i = self.map.picked()?;
        Some((i, s.suggestion(i)?.to_string()))
    }

    /// File the message picked, not yet filed, where it would go; or, all,
    /// mark every message shown that would go there too, and, marked, file
    /// them.
    fn file_where_suggested(&mut self, all: bool) -> bool {
        let (Some(s), Some((i, label))) = (self.map.snapshot.clone(), self.picked_suggestion())
        else {
            return false;
        };
        if !all {
            self.file_keys(vec![s.dots[i].key.clone()], &label);
            return true;
        }
        let now = jiff::Timestamp::now().as_second();
        let keys: Vec<crate::index::Key> = self
            .map
            .shown(now)
            .into_iter()
            .filter(|&j| s.suggestion(j) == Some(label.as_str()))
            .map(|j| s.dots[j].key.clone())
            .collect();
        if self.map.region.as_ref().is_some_and(|r| r.keys == keys) {
            self.file_keys(keys, &label);
        } else {
            self.status = format!(
                "{} would go to {label}, marked: Y files them, esc lets go",
                keys.len()
            );
            self.map.region = Some(map::Region { keys, boxed: None });
        }
        true
    }

    /// File the first of the suggestions too, or all of them: those for the
    /// message picked, if it is filed under no label yet, else those for the
    /// label last filed under.
    fn file_suggested(&mut self, all: bool) {
        if self.file_where_suggested(all) {
            return;
        }
        let (Some(s), Some(label)) = (self.map.snapshot.clone(), self.map.suggesting.clone())
        else {
            self.status =
                "nothing suggested: m files a message, and its like are suggested".to_string();
            return;
        };
        let keys: Vec<crate::index::Key> = self
            .suggestions()
            .into_iter()
            .take(if all { usize::MAX } else { 1 })
            .map(|(i, _)| s.dots[i].key.clone())
            .collect();
        if keys.is_empty() {
            self.status = format!("nothing more would go to {label}");
            return;
        }
        self.file_keys(keys, &label);
        if all {
            self.map.suggesting = None;
        }
    }

    /// Say the first suggestion is not one for its label: it is suggested
    /// no more, and pushed away from it.
    fn not_it(&mut self) {
        let Some(s) = self.map.snapshot.clone() else {
            return;
        };
        let (i, label) = match (self.picked_suggestion(), self.map.suggesting.clone()) {
            (Some(picked), _) => picked,
            (None, Some(label)) => match self.suggestions().first() {
                Some(&(i, _)) => (i, label),
                None => return,
            },
            (None, None) => return,
        };
        let kept = self
            .index
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .keep(vec![crate::index::Kept::NotIt {
                key: s.dots[i].key.clone(),
                label: label.clone(),
            }]);
        self.status = match kept {
            Ok(()) => format!("not {label}: {} is suggested no more", s.dots[i].subject),
            Err(e) => format!("cannot keep that: {e:#}"),
        };
    }

    /// Ask what to call the label of the message picked: its cluster's, or
    /// the last it was given.
    fn ask_rename_label(&mut self) {
        let Some(s) = self.map.snapshot.clone() else {
            return;
        };
        let label = self.map.picked().and_then(|i| {
            s.group_of(i)
                .and_then(|g| g.label.clone())
                .or_else(|| s.dots[i].labels.last().cloned())
        });
        match label {
            Some(l) => self.dialog = Dialog::RenameLabel(l.clone(), l),
            None => self.status = "the message picked has no label to rename".to_string(),
        }
    }

    /// Call the label `old` `new`, on every account that has it, and where
    /// letterbox alone keeps it; onto a label there is, the two are one.
    fn rename_label(&mut self, old: &str, new: &str) {
        if new.is_empty() || crate::index::fold(old) == crate::index::fold(new) && old == new {
            self.status = format!("kept {old}");
            return;
        }
        let Some(s) = self.map.snapshot.clone() else {
            return;
        };
        let folded = crate::index::fold(old);
        let accounts: std::collections::BTreeSet<usize> = s
            .dots
            .iter()
            .filter(|d| d.labels.iter().any(|l| crate::index::fold(l) == folded))
            .filter_map(|d| self.place_of(d).map(|p| p.0))
            .collect();
        for a in accounts {
            self.ask(
                a,
                Ask::RenameLabel {
                    old: old.to_string(),
                    new: new.to_string(),
                },
            );
        }
        let kept = self
            .index
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .keep(vec![crate::index::Kept::Renamed {
                from: old.to_string(),
                to: new.to_string(),
            }]);
        if let Err(e) = kept {
            self.status = format!("cannot rename {old}: {e:#}");
            return;
        }
        if self.map.suggesting.as_deref().map(crate::index::fold) == Some(folded) {
            self.map.suggesting = Some(new.to_string());
        }
        self.status = format!("renamed {old} to {new}");
    }

    /// Take what a server said of filing a message: a label it does not
    /// keep, letterbox keeps; one it would not take is no longer pending.
    fn filed(
        &mut self,
        a: usize,
        key: crate::index::Key,
        label: String,
        filed: Result<crate::store::Filed, String>,
    ) {
        let mut kept = Vec::new();
        match filed {
            Ok(f) if f.on_server => {}
            Ok(_) => {
                kept.push(crate::index::Kept::Label {
                    key: key.clone(),
                    label: label.clone(),
                    on: true,
                });
                kept.push(crate::index::Kept::Settled { key, label });
            }
            Err(e) => {
                self.status = format!("cannot file it under {label}: {}", first_line(&e));
                kept.push(crate::index::Kept::Settled { key, label });
            }
        }
        if !kept.is_empty() {
            let _ = self
                .index
                .lock()
                .unwrap_or_else(|e| e.into_inner())
                .keep(kept);
        }
        self.reload(a);
        self.map_pass(a);
    }

    /// Lay the map out again, off this thread, from where each message is,
    /// to be shown before it is kept.
    fn relayout(&mut self) {
        let Some(s) = self.map.snapshot.clone() else {
            return;
        };
        if s.at.is_none() || self.map.relaying {
            return;
        }
        self.map.relaying = true;
        self.pending += 1;
        self.status = "laying the map out again…".to_string();
        let replies = self.replies_to.clone();
        std::thread::spawn(move || {
            let at = map::relayout(&s);
            let _ = replies.send(Reply {
                account: usize::MAX,
                answer: Answer::Relaid {
                    lens: s.lens,
                    keys: s.model.keys.clone(),
                    at,
                },
            });
        });
    }

    /// Show the map laid out again, onto the map as it is now: mail come
    /// while it was laid out stays where it is, mail gone is let go.
    fn relaid(
        &mut self,
        lens: crate::index::Lens,
        keys: &[crate::index::Key],
        at: &[crate::model::Point],
    ) {
        self.map.relaying = false;
        let Some(s) = self.map.snapshot.clone() else {
            return;
        };
        let at = match map::onto(&s, keys, at) {
            Some(at) if s.lens == lens => at,
            _ => {
                self.status =
                    "the map was turned while it was laid out again: M-r again".to_string();
                return;
            }
        };
        let moved = map::moved(&s, &at);
        let faith = crate::model::faithfulness(&s.model, &at);
        self.status = format!(
            "laid out again, {moved} moved, {:.0}% faithful: ⏎ keeps it, esc drops it",
            faith * 100.0
        );
        self.map.preview = Some(at);
    }

    /// Keep the map as it was laid out again.
    fn keep_relayout(&mut self) {
        let (Some(s), Some(at)) = (self.map.snapshot.clone(), self.map.preview.take()) else {
            return;
        };
        // Laid out again, the clusters' nodes are put where they now are.
        self.map.pins.clear();
        let layout = s
            .model
            .keys
            .iter()
            .zip(&at)
            .map(|(k, p)| (k.clone(), p[0], p[1]))
            .collect();
        let kept = self
            .index
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .keep(vec![crate::index::Kept::Layout {
                lens: s.lens,
                at: layout,
            }]);
        self.status = match kept {
            Ok(()) => "the map, laid out again, kept".to_string(),
            Err(e) => format!("the map cannot keep its layout: {e:#}"),
        };
    }

    /// Narrow the map to the cluster of the anchor clicked; again, to all.
    /// The messages in an Inbox, to look through: newest first, by their
    /// place in the map; none sent from here.
    pub fn review_rows(&self) -> Vec<usize> {
        let Some(s) = &self.map.snapshot else {
            return Vec::new();
        };
        let mut rows: Vec<usize> = (0..s.dots.len())
            .filter(|&i| s.dots[i].system.inbox && !s.dots[i].system.sent)
            .collect();
        rows.sort_by_key(|&i| {
            let d = &s.dots[i];
            std::cmp::Reverse((d.date.unwrap_or(d.arrived), i))
        });
        rows
    }

    /// What is suggested for the message `i` of `s`: discarded, if mail
    /// like it is thrown away, as learnt; else filed where it would go;
    /// else kept.
    pub fn suggested(s: &map::Snapshot, i: usize) -> Choice {
        if s.dots[i].thrown.is_some() {
            return Choice::Discard;
        }
        s.suggestion(i)
            .map_or(Choice::Keep, |l| Choice::Move(l.to_string()))
    }

    /// What is to be done with the message `i` of the map: what was chosen,
    /// else what is suggested.
    pub fn choice(&self, i: usize) -> Choice {
        let Some(s) = &self.map.snapshot else {
            return Choice::Keep;
        };
        self.review
            .as_ref()
            .and_then(|r| r.chosen.get(&s.dots[i].key))
            .cloned()
            .unwrap_or_else(|| Self::suggested(s, i))
    }

    /// Open the Inbox to look through beside the map; or close it, what was
    /// chosen kept for when it is opened again.
    fn open_review(&mut self) {
        if let Some(r) = self.review.take() {
            self.review_kept = r.chosen;
            self.focus = Focus::List;
            self.map.moving = None;
            self.status = "the Inbox looked through, closed: i opens it as it was".to_string();
            return;
        }
        if self.map.mode != Mode::Map {
            self.show_map();
            if self.map.mode != Mode::Map {
                return;
            }
        }
        let first = self.review_rows().first().copied();
        self.review = Some(Review {
            chosen: std::mem::take(&mut self.review_kept),
            ..Review::default()
        });
        self.focus = Focus::Reader;
        self.pick_review(first);
        self.status =
            "the Inbox, to look through: m move, space keep, d discard, y as suggested, $ does it"
                .to_string();
    }

    /// Pick the message `i` of the map in the Inbox looked through, and on
    /// the map.
    fn pick_review(&mut self, i: Option<usize>) {
        let key = i.and_then(|i| Some(self.map.snapshot.as_ref()?.dots[i].key.clone()));
        if let Some(r) = &mut self.review {
            r.picked = key.clone();
            r.reading = false;
        }
        if key.is_some() {
            self.map.cursor = key;
        }
    }

    /// The message picked in the Inbox looked through, by its place in the
    /// map, and in it.
    fn review_at(&self) -> Option<(usize, usize)> {
        let s = self.map.snapshot.as_ref()?;
        let i = s.find(self.review.as_ref()?.picked.as_ref()?)?;
        let at = self.review_rows().iter().position(|&j| j == i)?;
        Some((i, at))
    }

    /// Pick the message `by` after the one picked, or before.
    fn review_step(&mut self, by: isize) {
        let rows = self.review_rows();
        if rows.is_empty() {
            return;
        }
        let at = self.review_at().map_or(0, |(_, at)| at as isize + by);
        self.pick_review(Some(rows[at.clamp(0, rows.len() as isize - 1) as usize]));
    }

    /// Choose `c` for the message `i`; then, `on`, go on to the next.
    fn choose(&mut self, i: usize, c: Choice, on: bool) {
        let Some(s) = self.map.snapshot.clone() else {
            return;
        };
        let key = s.dots[i].key.clone();
        if let Some(r) = &mut self.review {
            if c == Self::suggested(&s, i) {
                r.chosen.remove(&key);
            } else {
                r.chosen.insert(key, c);
            }
        }
        if on {
            self.review_step(1);
        }
    }

    /// Drop down the labels the message `i` may be moved to.
    fn open_menu(&mut self, i: usize) {
        let Some(key) = self.map.snapshot.as_ref().map(|s| s.dots[i].key.clone()) else {
            return;
        };
        if let Some(r) = &mut self.review {
            r.menu = Some(Menu {
                key,
                ..Menu::default()
            });
        }
        self.status = "move it to: type to narrow, ↑ ↓ pick, ⏎ moves it there, esc".to_string();
    }

    /// The labels the message dropped down for may be moved to: where it
    /// would go first, then the rest by how near they are to it by angle,
    /// the nearest few, or, typed, every one holding what is typed; a new
    /// one, if none is called that; and picking on the map.
    pub fn menu_items(&self) -> Vec<Pick> {
        let (Some(s), Some(menu)) = (
            self.map.snapshot.as_ref(),
            self.review.as_ref().and_then(|r| r.menu.as_ref()),
        ) else {
            return Vec::new();
        };
        let Some(i) = s.find(&menu.key) else {
            return Vec::new();
        };
        let typed = menu.typed.trim().to_lowercase();
        let first = s.suggestion(i).map(crate::index::fold);
        let mut labels: Vec<(String, f32)> = s
            .model
            .label_vectors
            .iter()
            .filter(|(folded, (name, _))| {
                typed.is_empty() || folded.contains(&typed) || name.to_lowercase().contains(&typed)
            })
            .map(|(_, (name, v))| (name.clone(), crate::model::degrees(&s.model.vectors[i], v)))
            .collect();
        labels.sort_by(|a, b| {
            let lead = |n: &str| Some(crate::index::fold(n)) != first;
            lead(&a.0)
                .cmp(&lead(&b.0))
                .then(a.1.total_cmp(&b.1))
                .then(a.0.cmp(&b.0))
        });
        let named = labels
            .iter()
            .any(|(n, _)| crate::index::fold(n) == crate::index::fold(&menu.typed));
        labels.truncate(if typed.is_empty() {
            MENU_NEAREST
        } else {
            MENU_TYPED
        });
        let mut items: Vec<Pick> = labels.into_iter().map(|(n, d)| Pick::Label(n, d)).collect();
        if !typed.is_empty() && !named {
            items.push(Pick::New(menu.typed.trim().to_string()));
        }
        items.push(Pick::OnMap);
        items
    }

    /// A key while the labels are dropped down: what is typed narrows them.
    fn menu_key(&mut self, key: KeyEvent) {
        let count = self.menu_items().len();
        let Some(menu) = self.review.as_mut().and_then(|r| r.menu.as_mut()) else {
            return;
        };
        let ctrl = key.modifiers.contains(KeyModifiers::CONTROL);
        match key.code {
            KeyCode::Esc => {
                self.review.as_mut().expect("open").menu = None;
                self.status = "not moved".to_string();
            }
            KeyCode::Char('g') if ctrl => {
                self.review.as_mut().expect("open").menu = None;
                self.status = "not moved".to_string();
            }
            // Round, past either end.
            KeyCode::Down => menu.at = (menu.at + 1) % count.max(1),
            KeyCode::Char('n') if ctrl => menu.at = (menu.at + 1) % count.max(1),
            KeyCode::Up => menu.at = (menu.at + count.max(1) - 1) % count.max(1),
            KeyCode::Char('p') if ctrl => menu.at = (menu.at + count.max(1) - 1) % count.max(1),
            KeyCode::Enter => {
                let at = menu.at;
                self.pick_item(at);
            }
            _ => {
                if edit_line(&mut menu.typed, key) {
                    menu.at = 0;
                }
            }
        }
    }

    /// Pick the item `at` of the labels dropped down: moved there, to be
    /// done with the rest; or, picking on the map, where it goes asked there.
    fn pick_item(&mut self, at: usize) {
        let item = self.menu_items().get(at).cloned();
        let Some(menu) = self.review.as_mut().and_then(|r| r.menu.take()) else {
            return;
        };
        let Some(i) = self.map.snapshot.as_ref().and_then(|s| s.find(&menu.key)) else {
            return;
        };
        match item {
            Some(Pick::Label(name, _) | Pick::New(name)) => {
                self.status = format!("to go to {name} · $ does it, with the rest");
                self.choose(i, Choice::Move(name), true);
            }
            Some(Pick::OnMap) => self.pick_on_map(i),
            None => {}
        }
    }

    /// Pick where the message `i` goes on the map: a click on a cluster,
    /// or ⏎ on one.
    fn pick_on_map(&mut self, i: usize) {
        let Some(key) = self.map.snapshot.as_ref().map(|s| s.dots[i].key.clone()) else {
            return;
        };
        if let Some(r) = &mut self.review {
            r.relabel = Some(key.clone());
        }
        self.map.cursor = Some(key.clone());
        self.map.moving = Some(vec![key]);
        self.focus = Focus::List;
        self.status = "where it goes: a click on a cluster, or ⏎ on one; N a new label; esc stops"
            .to_string();
    }

    /// Move the message `i` under a new label, its name asked: chosen, to be
    /// made as it is filed.
    fn choose_new(&mut self, i: usize) {
        let Some(key) = self.map.snapshot.as_ref().map(|s| s.dots[i].key.clone()) else {
            return;
        };
        if let Some(r) = &mut self.review {
            r.relabel = Some(key.clone());
        }
        self.map.moving = Some(vec![key]);
        self.ask_new_label();
    }

    /// Do what a key does in the Inbox looked through, with the focus on
    /// it; whether it was its to do.
    fn review_act(&mut self, act: Act) -> bool {
        if self.focus != Focus::Reader || self.map.mode != Mode::Map {
            return false;
        }
        let Some(r) = &mut self.review else {
            return false;
        };
        // Reading one, esc goes back to the rest; what may be done with it
        // is chosen as it is beside the rest, and the rest shown again.
        if r.reading {
            match act {
                Act::Back => r.reading = false,
                Act::Review => self.open_review(),
                Act::Trash | Act::Move | Act::FileFirst | Act::NewLabel => {
                    r.reading = false;
                    return self.review_act(act);
                }
                _ => return false,
            }
            return true;
        }
        let picked = self.review_at().map(|(i, _)| i);
        match (act, picked) {
            (Act::Down, _) => self.review_step(1),
            (Act::Up, _) => self.review_step(-1),
            (Act::Top, _) => self.review_step(isize::MIN / 2),
            (Act::End, _) => self.review_step(isize::MAX / 2),
            (Act::Left | Act::Right, Some(i)) => {
                let label = match self.choice(i) {
                    Choice::Move(l) => Some(l),
                    _ => self
                        .map
                        .snapshot
                        .as_ref()
                        .and_then(|s| s.suggestion(i))
                        .map(str::to_string),
                };
                let mut all: Vec<Choice> = label.into_iter().map(Choice::Move).collect();
                all.extend([Choice::Keep, Choice::Discard]);
                let now = self.choice(i);
                let at = all.iter().position(|c| *c == now).unwrap_or(0) as isize;
                let by = if act == Act::Right { 1 } else { -1 };
                let next = all[(at + by).rem_euclid(all.len() as isize) as usize].clone();
                self.choose(i, next, false);
            }
            (Act::Move, Some(i)) => self.open_menu(i),
            (Act::NewLabel, Some(i)) => self.choose_new(i),
            (Act::ScreenDown, Some(i)) => self.choose(i, Choice::Keep, true),
            (Act::Trash, Some(i)) => self.choose(i, Choice::Discard, true),
            (Act::FileFirst, Some(i)) => {
                let suggested = self
                    .map
                    .snapshot
                    .as_ref()
                    .map_or(Choice::Keep, |s| Self::suggested(s, i));
                self.choose(i, suggested, true);
            }
            (Act::Open, Some(_)) => {
                self.read_dot();
                if let Some(r) = &mut self.review {
                    r.reading = true;
                }
            }
            (Act::Apply, _) => self.ask_apply(),
            (Act::Review, _) => self.open_review(),
            // esc goes back to the map, all of it kept.
            (Act::Back, _) => self.focus = Focus::List,
            (Act::Seen, Some(_)) => self.dot_flag(Flag::Seen),
            (Act::Flag, Some(_)) => self.dot_flag(Flag::Flagged),
            (Act::Archive, _) => {
                self.status =
                    "here, m moves it, space keeps it, d discards it; $ does it all".to_string()
            }
            (
                Act::Left
                | Act::Right
                | Act::Move
                | Act::NewLabel
                | Act::ScreenDown
                | Act::Trash
                | Act::FileFirst
                | Act::Open,
                None,
            ) => {}
            _ => return false,
        }
        true
    }

    /// How many of the Inbox looked through are to be filed, discarded and
    /// kept.
    fn review_counts(&self) -> (usize, usize, usize) {
        let (mut filed, mut discarded, mut kept) = (0, 0, 0);
        for i in self.review_rows() {
            match self.choice(i) {
                Choice::Move(_) => filed += 1,
                Choice::Discard => discarded += 1,
                Choice::Keep => kept += 1,
            }
        }
        (filed, discarded, kept)
    }

    /// Ask whether to do what was chosen in the Inbox looked through.
    fn ask_apply(&mut self) {
        let (filed, discarded, kept) = self.review_counts();
        if filed + discarded == 0 {
            self.status = "nothing to do: each is kept in the Inbox".to_string();
            return;
        }
        if let Some(r) = &mut self.review {
            r.asking = true;
        }
        self.status =
            format!("File {filed}, discard {discarded}, keep {kept} in the Inbox? (y or n)");
    }

    /// Do what was chosen in the Inbox looked through: each to be moved
    /// filed under its label, each to be discarded moved to the Trash.
    fn apply_review(&mut self) {
        let Some(s) = self.map.snapshot.clone() else {
            return;
        };
        let mut moves: std::collections::BTreeMap<String, Vec<crate::index::Key>> =
            std::collections::BTreeMap::new();
        let mut discard = Vec::new();
        let mut kept = Vec::new();
        for i in self.review_rows() {
            let key = s.dots[i].key.clone();
            let chosen = self
                .review
                .as_ref()
                .is_some_and(|r| r.chosen.contains_key(&key));
            match self.choice(i) {
                Choice::Move(label) => moves.entry(label).or_default().push(key),
                Choice::Discard => discard.push(key),
                // Kept by choice, not as it was: learnt as not thrown away.
                Choice::Keep if chosen => kept.push(key),
                Choice::Keep => {}
            }
        }
        self.teach(&kept, false);
        if let Some(r) = &mut self.review {
            r.relabel = None;
            r.chosen.clear();
        }
        self.review_kept.clear();
        let filed: usize = moves.values().map(Vec::len).sum();
        for (label, keys) in moves {
            self.file_keys(keys, &label);
        }
        self.map.suggesting = None;
        let mut said = format!("{filed} filed");
        if !discard.is_empty() {
            self.map.region = Some(map::Region {
                keys: discard,
                boxed: None,
            });
            self.region_trash();
            said = format!("{said} · {}", self.status);
        }
        self.status = said;
        let first = self.review_rows().first().copied();
        self.pick_review(first);
    }

    /// A click on a message in the Inbox looked through picks it; on the
    /// one picked, reads it.
    fn review_click(&mut self, row: usize) {
        let Some(&i) = self.review_rows().get(row) else {
            return;
        };
        self.focus = Focus::Reader;
        if self.review_at().is_some_and(|(j, _)| j == i) {
            self.review_act(Act::Open);
        } else {
            self.pick_review(Some(i));
        }
    }

    /// A click on one of what may be done with a message looked through.
    fn choice_click(&mut self, row: usize, which: u8) {
        let Some(&i) = self.review_rows().get(row) else {
            return;
        };
        self.focus = Focus::Reader;
        self.pick_review(Some(i));
        match which {
            0 => self.open_menu(i),
            1 => self.choose(i, Choice::Keep, false),
            2 => self.choose(i, Choice::Discard, false),
            _ => self.choose_new(i),
        }
    }

    /// A click on a branch of a cluster's tree picks it; on the one
    /// picked, goes into it, or reads it; on the cluster, goes back to
    /// the clusters.
    fn branch_click(&mut self, row: usize) {
        self.focus = Focus::List;
        let Some(path) = self.map.branches.get(row).cloned() else {
            return;
        };
        // The cluster itself: back to the clusters.
        if path.is_empty() {
            return self.map.close_tree();
        }
        let now = jiff::Timestamp::now().as_second();
        let picked = self.map.tree_picked(now).map(|(_, p)| p);
        if picked.as_ref() == Some(&path) {
            self.map_act(Act::Open);
        } else {
            self.map.pick_branch(&path, now);
            if self.side_by_side {
                self.due = Some((Instant::now(), Due::Message));
            }
        }
    }

    fn anchor_click(&mut self, id: u32) {
        self.focus = Focus::List;
        if self.map.moving.is_some() {
            return self.move_onto(id);
        }
        if self.map.clusters {
            return self.map.open_node(id);
        }
        self.map.focused = if self.map.focused == Focused::Cluster(id) {
            Focused::All
        } else {
            Focused::Cluster(id)
        };
    }
}

/// `word` with a capital, as a label is given one.
fn capitalised(word: &str) -> String {
    let mut chars = word.chars();
    match chars.next() {
        Some(c) => c.to_uppercase().chain(chars).collect(),
        None => String::new(),
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

/// `key` as the table reads it: a letter said in lower case with Shift
/// alone, as some terminals say it once keys are told apart, in upper case,
/// as the rest say it.
fn capital(key: KeyEvent) -> KeyEvent {
    match key.code {
        KeyCode::Char(c) if c.is_lowercase() && key.modifiers == KeyModifiers::SHIFT => {
            let mut up = c.to_uppercase();
            match (up.next(), up.next()) {
                (Some(u), None) => KeyEvent {
                    code: KeyCode::Char(u),
                    ..key
                },
                _ => key,
            }
        }
        _ => key,
    }
}

/// The files `text` names, as a terminal pastes those dropped on it:
/// their paths, each quoted or its spaces escaped, or their `file://`
/// addresses; none unless each is a file named from the root or home.
fn dropped(text: &str) -> Vec<PathBuf> {
    let mut words = Vec::new();
    let mut word = String::new();
    let mut quote: Option<char> = None;
    let mut chars = text.trim().chars();
    while let Some(c) = chars.next() {
        match quote {
            Some(q) if c == q => quote = None,
            Some(_) => word.push(c),
            None if c == '\\' => word.extend(chars.next()),
            None if c == '\'' || c == '"' => quote = Some(c),
            None if c.is_whitespace() => {
                if !word.is_empty() {
                    words.push(std::mem::take(&mut word));
                }
            }
            None => word.push(c),
        }
    }
    if !word.is_empty() {
        words.push(word);
    }
    let paths: Vec<PathBuf> = words
        .iter()
        .map(|w| match w.strip_prefix("file://") {
            Some(rest) => PathBuf::from(unescaped(rest)),
            None => compose::path_of(w),
        })
        .collect();
    if paths.iter().all(|p| p.is_absolute() && p.is_file()) {
        paths
    } else {
        Vec::new()
    }
}

/// `s` with each `%` and two hex digits the byte they stand for.
fn unescaped(s: &str) -> String {
    let bytes = s.as_bytes();
    let mut out = Vec::with_capacity(bytes.len());
    let mut i = 0;
    while i < bytes.len() {
        let hex = bytes
            .get(i + 1..i + 3)
            .and_then(|h| std::str::from_utf8(h).ok())
            .and_then(|h| u8::from_str_radix(h, 16).ok());
        match (bytes[i], hex) {
            (b'%', Some(b)) => {
                out.push(b);
                i += 3;
            }
            (b, _) => {
                out.push(b);
                i += 1;
            }
        }
    }
    String::from_utf8_lossy(&out).into_owned()
}

/// The map's index, its key made if it was not, where letterbox keeps it.
fn open_map_index() -> Result<Index, String> {
    Index::open(
        &crate::index::key::dir(),
        crate::secret::keepers,
        crate::secret::find,
        crate::secret::store,
    )
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
                    theme::TERMINAL_DARK
                } else {
                    (255, 255, 255)
                }
            }
            Color::Reset => {
                if dark {
                    theme::TERMINAL_DARK_TEXT
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
/// with control, as `<c-u>`, Alt, as `<m-g>`, or Super, as `<s-g>`;
/// anything else, each character typed.
pub fn keys(spec: &str) -> Vec<KeyEvent> {
    // `<c-s-r>` is `C-S-r`, `<s-tab>` is `S-tab`, `<enter>` is `enter`, as
    // the foot names them.
    if let Some(name) = spec.strip_prefix('<').and_then(|s| s.strip_suffix('>'))
        && let Some(key) = keymap::key_named(&match name {
            "s-tab" => "S-tab".to_string(),
            _ => name
                .replace("c-s-", "C-S-")
                .replace("c-", "C-")
                .replace("m-", "M-"),
        })
    {
        return vec![key];
    }
    spec.chars()
        .map(|c| KeyEvent::new(KeyCode::Char(c), KeyModifiers::NONE))
        .collect()
}
