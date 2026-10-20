//! The TUI: what is shown, and what keys do. Each account's store is asked
//! from its own thread; what it answers is taken in between keys, so the
//! screen is never kept waiting by a server.

use crate::mail::{self, Folder, Message, Summary};
use crate::store::{Flag, Store};
use crate::theme::{self, Theme};
use crate::ui;
use crate::worker::{self, Answer, Ask, Reply};
use ratatui::DefaultTerminal;
use ratatui::crossterm::event::{
    self, DisableMouseCapture, EnableMouseCapture, Event, KeyCode, KeyEvent, KeyEventKind,
    KeyModifiers, MouseButton, MouseEvent, MouseEventKind,
};
use ratatui::crossterm::execute;
use ratatui::layout::{Position, Rect};
use ratatui::widgets::ListState;
use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::Arc;
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
    pub folders: Vec<Folder>,
    pub state: State,
    asks: Sender<Ask>,
}

/// What the sidebar lists: each account, and its folders under it.
#[derive(Clone, Copy, PartialEq, Debug)]
pub enum Row {
    Account(usize),
    Folder(usize, usize),
}

/// The messages of the folder shown.
pub struct Listing {
    pub account: usize,
    pub folder: String,
    pub messages: Vec<Summary>,
    pub total: u32,
    pub loading: bool,
    pub error: Option<String>,
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
}

/// What waits for a pause in moving through a list.
#[derive(Clone, Copy, PartialEq)]
enum Due {
    Folder,
    Message,
}

pub struct App {
    pub accounts: Vec<Account>,
    pub rows: Vec<Row>,
    pub row: usize,
    pub listing: Option<Listing>,
    /// Folders shown before, as they were, to show again at once.
    listed: HashMap<(usize, String), (Vec<Summary>, u32)>,
    /// What is listed: the folder's messages, or those the search finds.
    pub shown: Vec<Summary>,
    pub list: ListState,
    pub query: String,
    pub searching: bool,
    /// What the server found for the query, once asked with ⏎.
    pub found: Option<Vec<Summary>>,
    pub reading: Option<Reading>,
    /// How far the message read is scrolled, and how many lines it has and
    /// shows, as last drawn.
    pub scroll: usize,
    pub lines: usize,
    pub height: usize,
    /// Whether the message read is on the screen beside the list.
    pub side_by_side: bool,
    read: HashMap<(usize, String, u32), Arc<Message>>,
    pub focus: Focus,
    pub theme: Theme,
    pub status: String,
    pub help: bool,
    pub targets: Vec<(Rect, Target)>,
    /// Whether to mark a message read when it is shown.
    pub mark_read: bool,
    /// Where the accounts are set up, to say so where there are none.
    pub config: PathBuf,
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

/// The terminal, as ratatui takes it, with the mouse's clicks and wheel.
fn take() -> DefaultTerminal {
    let terminal = ratatui::init();
    let _ = execute!(std::io::stdout(), EnableMouseCapture);
    terminal
}

fn give_back() {
    let _ = execute!(std::io::stdout(), DisableMouseCapture);
    ratatui::restore();
}

impl App {
    /// The app over `stores`, each an account by its name; their folders
    /// are asked for at once.
    pub fn new(stores: Vec<(String, Box<dyn Store>)>, config: PathBuf) -> Self {
        let (replies_to, replies) = mpsc::channel();
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
            read: HashMap::new(),
            focus: Focus::List,
            theme: Theme::of(theme::Name::default()),
            status: String::new(),
            help: false,
            targets: Vec::new(),
            mark_read: true,
            config,
            replies,
            replies_to,
            pending: 0,
            due: None,
            refreshed: Instant::now(),
            clicked: None,
            quit: false,
        };
        for (i, (name, store)) in stores.into_iter().enumerate() {
            let asks = worker::spawn(i, store, app.replies_to.clone());
            app.accounts.push(Account {
                name,
                folders: Vec::new(),
                state: State::Connecting,
                asks,
            });
            app.ask(i, Ask::Folders);
        }
        app.rebuild();
        app
    }

    fn ask(&mut self, account: usize, ask: Ask) {
        if self.accounts[account].asks.send(ask).is_ok() {
            self.pending += 1;
        }
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
        self.pending = self.pending.saturating_sub(1);
        let a = reply.account;
        match reply.answer {
            Answer::Folders(Ok(folders)) => {
                let first = self.accounts.iter().all(|x| x.folders.is_empty());
                self.accounts[a].folders = folders;
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
            Answer::Count { counts: Err(_), .. } | Answer::Skipped => {}
            Answer::Page {
                folder,
                offset,
                page,
            } => {
                let Some(l) = self
                    .listing
                    .as_mut()
                    .filter(|l| l.account == a && l.folder == folder)
                else {
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
                let current = self
                    .listing
                    .as_ref()
                    .is_some_and(|l| l.account == a && l.folder == folder);
                if !current || words != self.query.trim() {
                    return;
                }
                match found {
                    Ok(found) => {
                        self.status = format!("{} found in {}", found.len(), self.folder_name());
                        self.found = Some(found);
                    }
                    Err(e) => self.status = format!("cannot search: {}", first_line(&e)),
                }
                self.narrow();
            }
        }
    }

    fn folder_mut(&mut self, account: usize, path: &str) -> Option<&mut Folder> {
        self.accounts[account]
            .folders
            .iter_mut()
            .find(|f| f.path == path)
    }

    /// The name of the folder listed.
    pub fn folder_name(&self) -> String {
        self.listing
            .as_ref()
            .and_then(|l| {
                self.accounts[l.account]
                    .folders
                    .iter()
                    .find(|f| f.path == l.folder)
                    .map(|f| f.name.clone())
            })
            .unwrap_or_default()
    }

    /// The folder listed, as the sidebar has it.
    pub fn folder(&self) -> Option<&Folder> {
        let l = self.listing.as_ref()?;
        self.accounts[l.account]
            .folders
            .iter()
            .find(|f| f.path == l.folder)
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
            };
            changed |= *field != on;
            *field = on;
        };
        if let Some(l) = self
            .listing
            .as_mut()
            .filter(|l| l.account == account && l.folder == folder)
        {
            l.messages.iter_mut().for_each(&mut set);
            self.shown.iter_mut().for_each(&mut set);
            if let Some(found) = &mut self.found {
                found.iter_mut().for_each(&mut set);
            }
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
        for (a, account) in self.accounts.iter().enumerate() {
            self.rows.push(Row::Account(a));
            self.rows
                .extend((0..account.folders.len()).map(|i| Row::Folder(a, i)));
        }
        self.row = picked
            .and_then(|p| self.rows.iter().position(|r| *r == p))
            .unwrap_or(0);
    }

    pub fn current(&self) -> Option<&Summary> {
        self.shown.get(self.list.selected()?)
    }

    /// List the folder picked in the sidebar, as it was if it was listed
    /// before, and ask for its newest messages.
    fn open_folder(&mut self) {
        let Some(&Row::Folder(a, i)) = self.rows.get(self.row) else {
            return;
        };
        let Some(folder) = self.accounts[a].folders.get(i) else {
            return;
        };
        if !folder.selectable {
            return;
        }
        let path = folder.path.clone();
        if self
            .listing
            .as_ref()
            .is_some_and(|l| l.account == a && l.folder == path)
        {
            return;
        }
        if let Some(old) = self.listing.take() {
            self.listed
                .insert((old.account, old.folder), (old.messages, old.total));
        }
        let (messages, total) = self.listed.remove(&(a, path.clone())).unwrap_or_default();
        self.listing = Some(Listing {
            account: a,
            folder: path.clone(),
            messages,
            total,
            loading: true,
            error: None,
        });
        self.query.clear();
        self.found = None;
        self.searching = false;
        self.list.select(Some(0));
        *self.list.offset_mut() = 0;
        self.narrow();
        self.ask(
            a,
            Ask::Page {
                folder: path,
                offset: 0,
                n: PAGE,
            },
        );
    }

    /// Look in the folder listed again, and count every folder's messages.
    pub fn refresh(&mut self) {
        self.refreshed = Instant::now();
        if let Some(l) = &mut self.listing {
            l.loading = true;
            let (a, folder) = (l.account, l.folder.clone());
            self.ask(
                a,
                Ask::Page {
                    folder,
                    offset: 0,
                    n: PAGE,
                },
            );
        }
        for a in 0..self.accounts.len() {
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
        let uid = self.current().map(|s| s.uid);
        let messages = self
            .listing
            .as_ref()
            .map(|l| l.messages.as_slice())
            .unwrap_or_default();
        let words: Vec<String> = self
            .query
            .split_whitespace()
            .map(str::to_lowercase)
            .collect();
        self.shown = match &self.found {
            Some(found) if !words.is_empty() => found.clone(),
            _ if words.is_empty() => messages.to_vec(),
            _ => messages
                .iter()
                .filter(|m| {
                    let text = format!("{} {} {} {}", m.from, m.address, m.subject, m.preview)
                        .to_lowercase();
                    words.iter().all(|w| text.contains(w.as_str()))
                })
                .cloned()
                .collect(),
        };
        let at = uid
            .and_then(|u| self.shown.iter().position(|s| s.uid == u))
            .or(self.list.selected())
            .unwrap_or(0);
        self.list
            .select((!self.shown.is_empty()).then(|| at.min(self.shown.len() - 1)));
    }

    /// Read the message picked in the list: at once, if it was read before.
    fn read_picked(&mut self) {
        let Some(l) = &self.listing else { return };
        let Some(s) = self.current() else { return };
        let (a, folder, uid, seen) = (l.account, l.folder.clone(), s.uid, s.flags.seen);
        if self
            .reading
            .as_ref()
            .is_some_and(|r| r.account == a && r.folder == folder && r.uid == uid)
        {
            return;
        }
        self.scroll = 0;
        let mark = self.mark_read && !seen;
        let body = match self.read.get(&(a, folder.clone(), uid)).cloned() {
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
        let Some(l) = &self.listing else { return };
        let Some(s) = self.current() else { return };
        let (a, folder, uid) = (l.account, l.folder.clone(), s.uid);
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
        if !near || l.loading || self.found.is_some() || !self.query.is_empty() {
            return;
        }
        if l.messages.len() >= l.total as usize {
            return;
        }
        l.loading = true;
        let (a, folder, offset) = (l.account, l.folder.clone(), l.messages.len());
        self.ask(
            a,
            Ask::Page {
                folder,
                offset,
                n: PAGE,
            },
        );
    }

    pub fn key(&mut self, key: KeyEvent) {
        let ctrl = key.modifiers.contains(KeyModifiers::CONTROL);
        if ctrl && key.code == KeyCode::Char('c') {
            self.quit = true;
            return;
        }
        if self.help {
            self.help = false;
            return;
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
                KeyCode::Backspace => {
                    self.query.pop();
                    self.found = None;
                }
                KeyCode::Char('u') if ctrl => {
                    self.query.clear();
                    self.found = None;
                }
                KeyCode::Char(c) => {
                    self.query.push(c);
                    self.found = None;
                }
                _ => {}
            }
            self.narrow();
            return;
        }
        let page = self.height.saturating_sub(2).max(1) as isize;
        match key.code {
            KeyCode::Char('q') => self.quit = true,
            KeyCode::Char('?') => self.help = true,
            KeyCode::Char('/') => {
                self.searching = true;
                if self.focus == Focus::Side {
                    self.focus = Focus::List;
                }
            }
            KeyCode::Esc if self.focus == Focus::Reader => self.focus = Focus::List,
            KeyCode::Esc if !self.query.is_empty() => {
                self.query.clear();
                self.found = None;
                self.narrow();
            }
            KeyCode::Tab => {
                self.focus = match self.focus {
                    Focus::Side => Focus::List,
                    Focus::List if self.reading.is_some() => Focus::Reader,
                    Focus::List | Focus::Reader => Focus::Side,
                }
            }
            KeyCode::BackTab => {
                self.focus = match self.focus {
                    Focus::Side if self.reading.is_some() => Focus::Reader,
                    Focus::Side | Focus::Reader => Focus::List,
                    Focus::List => Focus::Side,
                }
            }
            KeyCode::Char('h') | KeyCode::Left => {
                self.focus = match self.focus {
                    Focus::Reader => Focus::List,
                    _ => Focus::Side,
                }
            }
            KeyCode::Char('l') | KeyCode::Right => match self.focus {
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
            KeyCode::Enter => match self.focus {
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
            KeyCode::Char('j') | KeyCode::Down => self.step(1),
            KeyCode::Char('k') | KeyCode::Up => self.step(-1),
            KeyCode::PageDown | KeyCode::Char(' ') if self.focus != Focus::Side => {
                self.scroll_by(page)
            }
            KeyCode::PageUp | KeyCode::Char('b') if self.focus != Focus::Side => {
                self.scroll_by(-page)
            }
            KeyCode::PageDown => self.step(5),
            KeyCode::PageUp => self.step(-5),
            KeyCode::Char('g') | KeyCode::Home => self.jump(false),
            KeyCode::Char('G') | KeyCode::End => self.jump(true),
            KeyCode::Char('n') => self.next(1),
            KeyCode::Char('p') => self.next(-1),
            KeyCode::Char('u') => self.toggle(Flag::Seen),
            KeyCode::Char('s') => self.toggle(Flag::Flagged),
            KeyCode::Char('r') => {
                self.refresh();
                self.status = "looking for new mail…".to_string();
            }
            KeyCode::Char('o') => self.open_in_browser(),
            KeyCode::Char('a') => self.save_attachments(),
            _ => {}
        }
    }

    fn search(&mut self) {
        let words = self.query.trim().to_string();
        let Some(l) = &self.listing else { return };
        if words.is_empty() {
            return;
        }
        let (a, folder) = (l.account, l.folder.clone());
        self.status = format!("searching {} on the server…", self.folder_name());
        self.ask(
            a,
            Ask::Search {
                folder,
                words,
                n: FOUND,
            },
        );
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
                    if matches!(self.rows[at], Row::Folder(..)) {
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
        let by = match m.kind {
            MouseEventKind::Down(MouseButton::Left) => return self.click(at),
            MouseEventKind::ScrollDown => 1,
            MouseEventKind::ScrollUp => -1,
            _ => return,
        };
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
        if self.help {
            self.help = false;
            return;
        }
        self.searching = false;
        match at {
            Some(Target::Search) => self.searching = true,
            Some(Target::Side) => self.focus = Focus::Side,
            Some(Target::Row(i)) => {
                self.focus = Focus::Side;
                if matches!(self.rows.get(i), Some(Row::Folder(..))) {
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
        let html = m.html.clone().unwrap_or_else(|| {
            format!(
                "<meta charset=utf-8><pre style='white-space: pre-wrap'>{}</pre>",
                escape(&m.text)
            )
        });
        let dir = std::env::temp_dir().join("letterbox");
        let path = dir.join(format!("{}.html", m.uid));
        let done = std::fs::create_dir_all(&dir)
            .and_then(|_| std::fs::write(&path, html))
            .map_err(anyhow::Error::from)
            .and_then(|_| open(&path));
        self.status = match done {
            Ok(()) => format!("opened {}", path.display()),
            Err(e) => format!("cannot open it: {e:#}"),
        };
    }

    /// Save the message's attachments to the downloads folder, each under
    /// a name not yet taken there.
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
        for (name, bytes) in files {
            let path = unused(&dir, &safe_name(&name));
            match std::fs::create_dir_all(&dir).and_then(|_| std::fs::write(&path, bytes)) {
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

/// `$LETTERBOX_DOWNLOADS`, else `~/Downloads`.
fn downloads() -> PathBuf {
    std::env::var_os("LETTERBOX_DOWNLOADS")
        .filter(|d| !d.is_empty())
        .map(PathBuf::from)
        .unwrap_or_else(|| {
            PathBuf::from(std::env::var_os("HOME").unwrap_or_default()).join("Downloads")
        })
}

/// An attachment's name as a file's: no folders in it, as a sender might
/// try, and nothing hidden.
fn safe_name(name: &str) -> String {
    let base = name.rsplit(['/', '\\']).next().unwrap_or_default();
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
    let named = match spec {
        "<enter>" => Some(KeyCode::Enter),
        "<esc>" => Some(KeyCode::Esc),
        "<tab>" => Some(KeyCode::Tab),
        "<space>" => Some(KeyCode::Char(' ')),
        "<up>" => Some(KeyCode::Up),
        "<down>" => Some(KeyCode::Down),
        "<left>" => Some(KeyCode::Left),
        "<right>" => Some(KeyCode::Right),
        "<backspace>" => Some(KeyCode::Backspace),
        _ => None,
    };
    if let Some(code) = named {
        return vec![KeyEvent::new(code, KeyModifiers::NONE)];
    }
    let ctrl: Option<Vec<char>> = spec
        .strip_prefix("<c-")
        .and_then(|s| s.strip_suffix('>'))
        .map(|s| s.chars().collect());
    if let Some(&[c]) = ctrl.as_deref() {
        return vec![KeyEvent::new(KeyCode::Char(c), KeyModifiers::CONTROL)];
    }
    spec.chars()
        .map(|c| KeyEvent::new(KeyCode::Char(c), KeyModifiers::NONE))
        .collect()
}
