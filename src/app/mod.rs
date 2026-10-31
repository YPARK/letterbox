//! The TUI: what is shown, and what keys do. Each account's store is asked
//! from its own thread; what it answers is taken in between keys, so the
//! screen is never kept waiting by a server.

mod bringing;
mod bulk;
mod checking;
mod choosing;
mod dialog;
mod files;
mod filing;
mod filtering;
mod folders;
mod form;
mod input;
mod listing;
mod mapview;
mod marking;
mod pages;
mod papers;
mod passing;
mod relabel;
mod replies;
mod review;
mod saving;
mod screen;
mod signing;
mod trashing;
mod undoing;
mod writing;
use self::bringing::*;
pub use self::bulk::*;
pub use self::checking::*;
pub use self::dialog::*;
pub use self::files::*;
pub use self::filtering::*;
pub use self::folders::*;
pub use self::form::*;
pub use self::input::*;
pub use self::listing::*;
pub use self::mapview::map_refuses;
use self::mapview::*;
pub use self::papers::*;
use self::passing::*;
pub use self::relabel::*;
pub use self::review::*;
pub use self::saving::*;
pub use self::screen::*;
pub use self::signing::*;
pub use self::trashing::*;
pub use self::writing::*;

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
use crate::store::{Flag, Store};
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
use std::collections::{HashMap, VecDeque};
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

/// Where the message read is put, as `\` turns it: beside the list, below
/// it, out of sight until a message is read, or over the list.
#[derive(Clone, Copy, PartialEq, Eq, Debug, Default)]
pub enum Placement {
    #[default]
    Right,
    Below,
    Hidden,
    Popup,
}

impl Placement {
    const ALL: [Placement; 4] = [Self::Right, Self::Below, Self::Hidden, Self::Popup];

    /// As it is written down, and said.
    pub fn name(self) -> &'static str {
        match self {
            Self::Right => "right",
            Self::Below => "below",
            Self::Hidden => "hidden",
            Self::Popup => "pop-up",
        }
    }

    /// Where it is, as the status says when it is turned to.
    pub fn said(self) -> &'static str {
        match self {
            Self::Right => "right of the list",
            Self::Below => "below the list",
            Self::Hidden => "hidden until ⏎ reads one",
            Self::Popup => "a pop-up over the list, until esc",
        }
    }

    /// What \ says once it puts the message read here.
    pub fn placed_said(self) -> String {
        format!("the message read: {}", self.said())
    }

    pub fn named(name: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|p| p.name() == name)
    }

    /// The one after it, as `\` turns them.
    pub fn next(self) -> Self {
        let at = Self::ALL.iter().position(|&p| p == self).unwrap_or(0);
        Self::ALL[(at + 1) % Self::ALL.len()]
    }
}

impl App {
    /// Where the message read is put on the panes as last drawn: as it
    /// was asked to be, but beside the list in a window too narrow for two
    /// panes, one at a time, and beside the map with the Inbox looked
    /// through, which follows its picks.
    pub fn placed(&self) -> Placement {
        if self.panes.width < crate::ui::MEDIUM
            || self.map.mode == Mode::Map && self.review.is_some()
        {
            Placement::Right
        } else {
            self.placement
        }
    }

    /// Below the list, as last drawn.
    pub fn stacked(&self) -> bool {
        self.placed() == Placement::Below
    }

    /// Whether < and > size the message read: below the list, or beside
    /// it; not alone, nor over it.
    pub fn resizable(&self) -> bool {
        self.sizing().is_some()
    }

    /// The message just read shown, alone if it is not with the list.
    pub(super) fn show_read(&mut self) {
        if !self.side_by_side {
            self.focus = Focus::Reader;
        }
    }
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
    /// Read on the map, not from the list: shown beside the one it was
    /// read from alone.
    pub on_map: bool,
}

/// What the terminal is given over to, or the system asked to do, once
/// the screen is drawn: a document presented by ohp, which takes the
/// terminal until it quits; a file opened by the system; a page in the
/// browser; an address put on the clipboard, by the terminal.
#[derive(Clone, Debug, PartialEq)]
pub enum Launch {
    /// A file presented by ohp; a PDF attached, which it may sign, with
    /// the message it came with.
    Ohp(PathBuf, Option<Presenting>),
    Open(PathBuf),
    Browse(String),
    Copy(String),
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
    /// Whether the message read is on the screen with the list, beside it
    /// or below it, and so read as the list is moved along; as last drawn.
    pub side_by_side: bool,
    /// The message read as last drawn, its width and colours, and its
    /// lines: wrapped once, not again each time the screen is drawn.
    pub drawn: Option<(Arc<Message>, usize, Theme, Vec<Line<'static>>)>,
    read: Kept,
    pub focus: Focus,
    pub theme: Theme,
    pub status: String,
    /// The passing word the status line says, if it says one.
    passing: Option<Passing>,
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
    /// What to do outside the screen, once it is drawn.
    pub launch: Option<Launch>,
    replies: Receiver<Reply>,
    replies_to: Sender<Reply>,
    /// Asks not yet answered.
    pending: usize,
    due: Option<(Instant, Due)>,
    refreshed: Instant,
    clicked: Option<(Instant, usize)>,
    pub quit: bool,
    /// The model the grammar is checked with, by its name on Hugging Face,
    /// how it is read, and the thread it is kept on once it is; and the
    /// changes it would make, gone through one by one.
    pub grammar_model: String,
    pub grammar_open: fn(&str) -> anyhow::Result<Box<dyn crate::grammar::Corrector>>,
    checker: Option<std::sync::mpsc::Sender<String>>,
    pub checking: Option<Checking>,
    /// Whether the grammar is checked before a message is sent, as it is
    /// where letterbox is built with it; whether it was, and it is asked if
    /// it is to go now; whether the next send goes as it is, its checking
    /// let go by C-c; and what is said of the grammar once it is sent.
    pub check_on_send: bool,
    pub sending_asked: bool,
    send_as_is: bool,
    send_note: Option<String>,
    checking_for_send: bool,
    /// The messages on their way to another account, by where each was.
    bringing: std::collections::BTreeMap<(usize, String, u32), Bringing>,
    /// What was asked of the servers, moving mail, followed till each is
    /// confirmed or failed; what was chosen in the Inbox looked through for
    /// what is being asked now; and whether what is asked now is done on its
    /// own, not followed.
    pub batch: crate::confirm::Batch,
    choosing: std::collections::BTreeMap<crate::index::Key, Choice>,
    quiet: bool,
    /// What was last moved, each as it was, for C-x u.
    undoable: undoing::Moves,
    /// A move remembered since the key pressed last: the rest of its are
    /// remembered with it.
    moves_begun: bool,
    /// The message C-x u put back, to be picked once it is listed again.
    pick_back: Option<undoing::PickBack>,
    /// How many the Inboxes held as U looked for new mail, till it is done.
    looking_new: Option<u32>,
    /// Moves not to remember: those that put back.
    quiet_moves: bool,
    /// A server's answer is taken: what it asks follows it, not a key, and
    /// is not remembered for C-x u.
    replying: bool,
    /// When the key pressed last was, in seconds since 1970, and the
    /// `Message-ID` of the message picked as it was pressed.
    input_at: i64,
    input_on: Option<String>,
    /// How long a message sent is held, to be taken back.
    pub send_hold: Duration,
    /// Asks that change mail, not yet answered: quitting while there are
    /// any is asked first.
    pub(super) changing: usize,
    /// C-x C-c was pressed while mail was being changed: y quits anyway.
    pub(super) quitting: bool,
    /// Over this many marked, the foot says so in the flag's colour.
    pub mark_big: usize,
    /// What was listed before what was found, for esc to go back to.
    before_found: Option<View>,
    /// What was done last that C-x u undoes.
    last: Option<Last>,
    /// What a label split or removed asked, followed for C-x u.
    refiling: Refiling,
    /// What was chosen in the Inbox looked through, not to be learnt when
    /// it is done: chosen for several together, or the filter's own guess
    /// taken as it was.
    untaught: std::collections::HashSet<crate::index::Key>,
    /// What looked like junk and was asked about, not to be asked again.
    junk_asked: std::collections::HashSet<crate::index::Key>,
    /// What looked like junk, held for J to show.
    junk: Vec<Junk>,
    /// What was thrown away, as last told of beside it, while tab goes
    /// between the two.
    parked: Option<Box<crate::filter::Debrief>>,
    /// When the wheel last went in or out a level on the map, and which way.
    wheeled: Option<(Instant, isize)>,
    /// The prefix pressed, `x` or `c`, the key after it waited for: C-x
    /// C-c quits, C-x u undoes; in the Inbox looked through, C-c C-c does
    /// what was chosen.
    prefix: Option<char>,
    /// The index the map reads, shared with each account's sync.
    pub index: Arc<Mutex<Index>>,
    /// Why there is no map, if there is none: as its key could not be had.
    pub map_off: Option<String>,
    /// The map is not begun: no key was ever made for it. It is begun once
    /// it is asked for, and agreed to.
    pub map_later: bool,
    /// The Inbox, being looked through beside the map.
    pub review: Option<Review>,
    /// The labels the message picked may be moved to, dropped down.
    pub menu: Option<Menu>,
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
    /// The folder the message's files were last saved in.
    pub save_dir: Option<PathBuf>,
    /// How wide the message read is, as it was dragged; else as the
    /// screen's width has it.
    pub reader_width: Option<u16>,
    /// Where the message read is put.
    pub placement: Placement,
    /// How tall the message read below the list is, as it was dragged;
    /// else half.
    pub reader_height: Option<u16>,
    /// The line before the message read is being dragged.
    pub resizing: bool,
    /// The row the left button was pressed on, to select as far as it is
    /// dragged.
    dragged_from: Option<Target>,
    /// Where the panes are, as last drawn.
    pub panes: Rect,
    /// How wide the message read was last drawn.
    pub read_width: u16,
    /// How tall the message read was last drawn.
    pub read_height: u16,
    /// The room a page of it has: within its border and padding, above
    /// its foot.
    pub page_room: (u16, u16),
    /// Where it is written down how the mail was looked at, the list or
    /// the map, to open as it was left; none to keep nothing.
    pub look_file: Option<PathBuf>,
    /// Whether new mail is thrown away on its own, and when what it did
    /// was last told of; and where that is kept, if anywhere.
    pub auto: crate::filter::Auto,
    pub filter_file: Option<PathBuf>,
    /// paperstack, once asked: run as it is first wanted, and kept.
    pub library: Arc<Mutex<Option<Box<dyn crate::paperstack::Library>>>>,
    /// What the terminal draws images with, once asked, as the message is
    /// first shown as its pages: none if it draws none. Asked then, not as
    /// letterbox starts, as a terminal that draws none keeps it waiting for
    /// an answer it never gives. And the message read as its pages, shown
    /// in place of its text.
    #[cfg(feature = "pages")]
    pub picker: Option<Option<ratatui_image::picker::Picker>>,
    #[cfg(feature = "pages")]
    pub paged: Option<crate::pages::Paged>,
    /// The message whose pages are being typeset; let go, they are not
    /// shown once they are.
    #[cfg(feature = "pages")]
    typesetting: Option<crate::pages::Place>,
    /// How many lists of papers paperstack has been asked of.
    papers_asked: u64,
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
                app.status = carry_out(&launch, &mut terminal);
                if let Launch::Ohp(_, Some(presented)) = &launch {
                    app.presented(presented);
                }
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
                    // The line before the message read, dragged, is drawn
                    // as it goes.
                    stale |= matches!(m.kind, MouseEventKind::Drag(_)) && app.resizing;
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
fn carry_out(launch: &Launch, terminal: &mut DefaultTerminal) -> String {
    match launch {
        Launch::Ohp(path, presenting) => {
            give_back();
            if let Some(dir) = presenting.as_ref().and_then(|p| p.report.parent()) {
                let _ = private_dir(dir);
            }
            let ran = std::process::Command::new("ohp")
                .arg(path)
                .envs(presenting.iter().flat_map(Presenting::env))
                .status();
            *terminal = take();
            match ran {
                Ok(s) if s.success() => String::new(),
                Ok(s) => format!("ohp: {s}"),
                Err(_) => match open(path) {
                    Ok(()) => {
                        "ohp is not installed (cargo install ohp): opened by the system".to_string()
                    }
                    Err(e) => format!("cannot open {}: {e:#}", path.display()),
                },
            }
        }
        Launch::Open(path) => match open(path) {
            Ok(()) => format!("opened {}", tilde(path)),
            Err(e) => format!("cannot open {}: {e:#}", path.display()),
        },
        Launch::Browse(url) => {
            oauth::browse(url);
            "opened in the browser".to_string()
        }
        Launch::Copy(text) => {
            // OSC 52: the terminal puts it on the clipboard, here or over ssh.
            use base64::Engine;
            let code = base64::engine::general_purpose::STANDARD.encode(text);
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
            passing: None,
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
            launch: None,
            replies,
            replies_to,
            pending: 0,
            due: None,
            refreshed: Instant::now(),
            clicked: None,
            quit: false,
            grammar_model: crate::grammar::DEFAULT.to_string(),
            grammar_open: crate::grammar::open,
            checker: None,
            checking: None,
            // Never in tests, which would fetch the model: those of it say so.
            check_on_send: cfg!(feature = "grammar") && !cfg!(test),
            sending_asked: false,
            send_as_is: false,
            send_note: None,
            checking_for_send: false,
            bringing: std::collections::BTreeMap::new(),
            batch: crate::confirm::Batch::default(),
            choosing: std::collections::BTreeMap::new(),
            quiet: false,
            // Tests send at once, but those of the hold itself.
            send_hold: if cfg!(test) {
                Duration::ZERO
            } else {
                writing::SEND_HOLD
            },
            undoable: undoing::Moves::default(),
            moves_begun: false,
            pick_back: None,
            looking_new: None,
            quiet_moves: false,
            replying: false,
            input_at: 0,
            input_on: None,
            changing: 0,
            quitting: false,
            mark_big: MARK_BIG,
            last: None,
            before_found: None,
            refiling: Refiling::default(),
            untaught: Default::default(),
            junk_asked: Default::default(),
            junk: Vec::new(),
            parked: None,
            wheeled: None,
            prefix: None,
            index: Arc::new(Mutex::new(index)),
            map_off,
            map_later,
            review: None,
            review_kept: Default::default(),
            menu: None,
            twins: Vec::new(),
            open_map: open_map_index,
            syncs: Vec::new(),
            mapping: String::new(),
            map: Map::default(),
            map_corpus: None,
            attach_dir: None,
            save_dir: None,
            reader_width: None,
            placement: Placement::Right,
            reader_height: None,
            resizing: false,
            dragged_from: None,
            panes: Rect::default(),
            read_width: 0,
            read_height: 0,
            page_room: (0, 0),
            look_file: None,
            auto: Default::default(),
            filter_file: None,
            library: Default::default(),
            #[cfg(feature = "pages")]
            picker: None,
            #[cfg(feature = "pages")]
            paged: None,
            #[cfg(feature = "pages")]
            typesetting: None,
            papers_asked: 0,
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
        let job = self.job_of(account, &ask);
        // As it is before it goes, for C-x u: remembered once it is sent.
        let noted = self.note_move(account, &ask, job.as_ref().and_then(|j| j.key.clone()));
        if let Some(job) = job {
            self.batch.ask(job);
        }
        let changes = ask.changes_mail();
        let sent = self.accounts[account]
            .asks
            .as_ref()
            .is_some_and(|s| s.send(ask).is_ok());
        if sent {
            self.pending += 1;
            self.changing += usize::from(changes);
            if let Some(noted) = noted {
                self.remember(noted);
            }
        }
    }

    /// Ask, not remembering what it moves for C-x u: it puts back.
    fn ask_quietly(&mut self, account: usize, ask: Ask) {
        let quiet = std::mem::replace(&mut self.quiet_moves, true);
        self.ask(account, ask);
        self.quiet_moves = quiet;
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
        let mut any = self.tick_held();
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
}

// ---- the map ----

impl App {}

fn first_line(s: &str) -> &str {
    s.lines().next().unwrap_or_default()
}

fn escape(s: &str) -> String {
    s.replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
}

fn tilde(path: &std::path::Path) -> String {
    let home = PathBuf::from(std::env::var_os("HOME").unwrap_or_default());
    match path.strip_prefix(&home) {
        Ok(rest) if !home.as_os_str().is_empty() => format!("~/{}", rest.display()),
        _ => path.display().to_string(),
    }
}
