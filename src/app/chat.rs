//! Slack beside the mail: each workspace's conversations in the sidebar, the
//! busiest first, a few shown and the rest folded but for those with anything
//! unread; one read in the list's place, and a thread in the message's.
//! Reading marks nothing read in Slack. What you write, `r`, goes as you send
//! it, C-⏎: Slack is for quick messages. Nothing else sends.
//! While a conversation is read, the keys that act on mail do nothing: the
//! mail behind it is out of sight.
use super::*;
use crate::slack::Slack;
use crate::slack::reading::Message as Said;
use crate::slack::worker::{self as slack_worker, Answer as Heard, Ask as SlackAsk};

#[derive(Default)]
pub struct Jump {
    pub query: String,
    pub at: usize,
}

/// How many conversations the jump box shows.
pub const FOUND: usize = 10;

/// The two parts of a workspace in Slack's column.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Part {
    Channels,
    /// Direct messages, and messages to a few people together.
    People,
}

impl Part {
    fn of(kind: crate::slack::Kind) -> Part {
        match kind {
            crate::slack::Kind::Direct | crate::slack::Kind::Group => Part::People,
            _ => Part::Channels,
        }
    }
}

/// A row of Slack's column: a workspace's heading; the heading of its
/// channels or its people; a conversation of it, by its place; and the row
/// that shows every one, or a few again.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum ChatRow {
    Head(usize),
    Section(usize, Part),
    Talk(usize, usize),
    More(usize),
}

impl ChatRow {
    /// Whether the row may be picked: not a heading.
    fn picks(&self) -> bool {
        matches!(self, ChatRow::Talk(..) | ChatRow::More(_))
    }
}

/// How many conversations of a workspace are shown, folded.
pub const SHOWN: usize = 12;
/// How often the conversation read is read again.
const READ_AGAIN: Duration = Duration::from_secs(30);
/// How often the conversations, and what they have unread, are read again;
/// and while what happens is heard as it does, which keeps the counts.
const COUNT_AGAIN: Duration = Duration::from_secs(5 * 60);
const COUNT_AGAIN_LIVE: Duration = Duration::from_secs(30 * 60);

/// A workspace: its name, its conversations, the busiest first, and what
/// each has unread, once counted.
pub struct Space {
    pub name: String,
    pub talks: Vec<Talk>,
    pub loading: bool,
    pub error: Option<String>,
    /// Whether every conversation is shown, not a few.
    pub unfolded: bool,
    /// Whether what happens is heard as it does, its socket open.
    pub live: bool,
    asks: Sender<SlackAsk>,
    asked: Instant,
}

pub struct Talk {
    pub channel: crate::slack::Channel,
    pub unread: Option<u32>,
}

/// The conversation read: its messages, the oldest first, the one picked,
/// and the thread read, if one is.
pub struct Open {
    pub space: usize,
    pub id: String,
    pub name: String,
    pub messages: Vec<Said>,
    pub loading: bool,
    pub error: Option<String>,
    pub picked: Option<usize>,
    pub thread: Option<OpenThread>,
    asked: Instant,
    /// What a search found, listed in its place: where each message is.
    pub hits: Option<Vec<crate::slack::reading::Hit>>,
    /// The message to pick once the conversation is read: one gone to.
    pick_ts: Option<String>,
}

pub struct OpenThread {
    pub ts: String,
    pub messages: Vec<Said>,
    pub loading: bool,
    pub error: Option<String>,
    pub scroll: usize,
}

/// A message being written: to which conversation, in which thread, if a
/// reply; what is written; whether it is on its way; and why it did not go,
/// if it did not.
pub struct Writing {
    pub space: usize,
    pub channel: String,
    pub name: String,
    pub thread: Option<String>,
    pub buffer: crate::edit::Buffer,
    prefix: Option<char>,
    pub sending: bool,
    pub error: Option<String>,
}

pub struct Chat {
    pub spaces: Vec<Space>,
    /// Slack's column: its rows, the one picked, and whether it is shown.
    pub rows: Vec<ChatRow>,
    pub row: usize,
    pub shown: bool,
    /// A conversation being gone to by name, C-k: what is typed, and which
    /// of those found is picked.
    pub jump: Option<Jump>,
    /// Slack being searched, `/`: what is typed.
    pub search: Option<String>,
    pub open: Option<Open>,
    pub writing: Option<Writing>,
    replies: Receiver<slack_worker::Reply>,
    replies_to: Sender<slack_worker::Reply>,
    /// Answers asked for and not yet had.
    pending: usize,
}

impl Default for Chat {
    fn default() -> Self {
        let (replies_to, replies) = mpsc::channel();
        Chat {
            spaces: Vec::new(),
            rows: Vec::new(),
            row: 0,
            shown: false,
            jump: None,
            search: None,
            open: None,
            writing: None,
            replies,
            replies_to,
            pending: 0,
        }
    }
}

impl Chat {
    /// The conversations of `space` shown in the sidebar, by their place: the
    /// busiest few, any with anything unread, and the one read; or all.
    pub fn shown(&self, space: usize) -> Vec<usize> {
        let s = &self.spaces[space];
        let open = self
            .open
            .as_ref()
            .filter(|o| o.space == space)
            .map(|o| o.id.as_str());
        (0..s.talks.len())
            .filter(|&i| {
                let talk = &s.talks[i];
                s.unfolded
                    || i < SHOWN
                    || talk.unread.is_some_and(|n| n > 0)
                    || Some(talk.channel.id.as_str()) == open
            })
            .collect()
    }

    fn ask(&mut self, space: usize, ask: SlackAsk) {
        // Conversations are answered twice: as they are, and once counted.
        let whole = matches!(ask, SlackAsk::Conversations);
        self.pending += if whole { 2 } else { 1 };
        if let Some(s) = self.spaces.get_mut(space) {
            if whole {
                s.asked = Instant::now();
            }
            if s.asks.send(ask).is_err() {
                self.pending = 0;
            }
        }
    }
}

impl App {
    /// Read Slack's workspace `name` too, by `slack`.
    pub fn chat_add(&mut self, name: String, slack: Slack, app: Option<String>) {
        let space = self.chat.spaces.len();
        let asks = slack_worker::spawn(space, slack, app, self.chat.replies_to.clone());
        self.chat.spaces.push(Space {
            name,
            talks: Vec::new(),
            loading: true,
            error: None,
            unfolded: false,
            live: false,
            asks,
            asked: Instant::now(),
        });
        self.chat.ask(space, SlackAsk::Conversations);
        self.chat_rebuild();
    }

    /// The rows of Slack's column: each workspace's heading, its channels
    /// and its people, each under their own heading, and the row that shows
    /// every one of them, or a few again.
    fn chat_rows(&self) -> Vec<ChatRow> {
        let mut rows = Vec::new();
        for (s, space) in self.chat.spaces.iter().enumerate() {
            rows.push(ChatRow::Head(s));
            let shown = self.chat.shown(s);
            for part in [Part::Channels, Part::People] {
                let these: Vec<usize> = shown
                    .iter()
                    .copied()
                    .filter(|&i| Part::of(space.talks[i].channel.kind) == part)
                    .collect();
                if !these.is_empty() {
                    rows.push(ChatRow::Section(s, part));
                    rows.extend(these.into_iter().map(|i| ChatRow::Talk(s, i)));
                }
            }
            if space.unfolded || shown.len() < space.talks.len() {
                rows.push(ChatRow::More(s));
            }
        }
        rows
    }

    /// Slack's column again, the row picked kept; else its first conversation.
    pub(super) fn chat_rebuild(&mut self) {
        let picked = self.chat.rows.get(self.chat.row).copied();
        self.chat.rows = self.chat_rows();
        self.chat.row = picked
            .and_then(|p| self.chat.rows.iter().position(|r| *r == p))
            .or_else(|| self.chat.rows.iter().position(ChatRow::picks))
            .unwrap_or(0);
    }

    /// Whether Slack's column is shown: asked for, and something to show.
    pub fn chat_shown(&self) -> bool {
        self.chat.shown && !self.chat.spaces.is_empty()
    }

    /// Whether the message's pane has something to go to: a message read,
    /// or a conversation, whose thread is read there.
    pub(super) fn has_reader(&self) -> bool {
        self.reading.is_some() || self.chat.open.is_some()
    }

    /// `K`: Slack's column shown, and gone to; or hidden again.
    pub(super) fn toggle_chat(&mut self) {
        if self.chat.spaces.is_empty() {
            self.status = "no Slack workspace yet: letterbox slack add".to_string();
            return;
        }
        self.chat.shown = !self.chat.shown;
        if self.chat.shown {
            self.focus = Focus::Chat;
            self.status = "Slack: ⏎ reads one, K hides it".to_string();
        } else if self.focus == Focus::Chat {
            self.focus = Focus::Side;
        }
    }

    /// Move through Slack's column, past its headings.
    pub(super) fn chat_step(&mut self, by: isize) {
        let rows = &self.chat.rows;
        let mut at = self.chat.row;
        let mut moved = 0;
        while moved < by.unsigned_abs() {
            let Some(next) = at
                .checked_add_signed(by.signum())
                .filter(|&n| n < rows.len())
            else {
                break;
            };
            at = next;
            if rows[at].picks() {
                self.chat.row = at;
                moved += 1;
            }
        }
    }

    /// The conversation `i` of `space` read in the list's place, from the map
    /// too, and gone to; picked in the column, where it is.
    fn read_talk(&mut self, space: usize, i: usize) {
        if self.map.mode == Mode::Map {
            self.looked_now();
            self.map.mode = Mode::List;
            self.map.region = None;
        }
        self.open_talk(space, i);
        self.chat_rebuild();
        if let Some(at) = self
            .chat
            .rows
            .iter()
            .position(|r| *r == ChatRow::Talk(space, i))
        {
            self.chat.row = at;
        }
        self.focus = Focus::List;
    }

    /// The row picked in Slack's column opened: a conversation read in the
    /// list's place; a heading or more, every one shown.
    fn chat_open_row(&mut self) {
        match self.chat.rows.get(self.chat.row).copied() {
            Some(ChatRow::Talk(s, i)) => self.read_talk(s, i),
            Some(ChatRow::Head(s) | ChatRow::More(s)) => self.chat_fold(s),
            _ => {}
        }
    }

    /// C-k: a conversation gone to by name.
    pub(super) fn open_jump(&mut self) {
        if self.chat.spaces.is_empty() {
            self.status = "no Slack workspace yet: letterbox slack add".to_string();
            return;
        }
        self.chat.jump = Some(Jump::default());
    }

    /// The conversations whose names hold what is typed, any case, every one
    /// of each workspace, folded or not: those it begins first, then those
    /// with anything unread, then the busiest; by workspace and place.
    pub fn jump_found(&self) -> Vec<(usize, usize)> {
        let Some(jump) = &self.chat.jump else {
            return Vec::new();
        };
        let asked = jump.query.trim().to_lowercase();
        let mut found: Vec<(usize, usize, bool, bool, i64)> = Vec::new();
        for (s, space) in self.chat.spaces.iter().enumerate() {
            for (i, talk) in space.talks.iter().enumerate() {
                let name = talk.channel.name.to_lowercase();
                if name.contains(&asked) {
                    let unread = talk.unread.is_some_and(|n| n > 0);
                    found.push((s, i, name.starts_with(&asked), unread, talk.channel.updated));
                }
            }
        }
        found.sort_by_key(|&(_, _, begins, unread, updated)| {
            (!begins, !unread, std::cmp::Reverse(updated))
        });
        found
            .into_iter()
            .take(FOUND)
            .map(|(s, i, ..)| (s, i))
            .collect()
    }

    /// A key while a conversation is gone to by name.
    pub(super) fn jump_key(&mut self, key: KeyEvent) {
        let ctrl = key.modifiers.contains(KeyModifiers::CONTROL);
        let found = self.jump_found();
        let Some(jump) = &mut self.chat.jump else {
            return;
        };
        match key.code {
            KeyCode::Esc => self.chat.jump = None,
            KeyCode::Char('g') if ctrl => self.chat.jump = None,
            KeyCode::Enter => {
                if let Some(&(s, i)) = found.get(jump.at) {
                    self.chat.jump = None;
                    self.read_talk(s, i);
                }
            }
            KeyCode::Down => jump.at = (jump.at + 1).min(found.len().saturating_sub(1)),
            KeyCode::Char('n') if ctrl => {
                jump.at = (jump.at + 1).min(found.len().saturating_sub(1))
            }
            KeyCode::Up => jump.at = jump.at.saturating_sub(1),
            KeyCode::Char('p') if ctrl => jump.at = jump.at.saturating_sub(1),
            _ => {
                if edit_line(&mut jump.query, key) {
                    jump.at = 0;
                }
            }
        }
    }

    /// A click on a row of Slack's column.
    pub(super) fn chat_click(&mut self, at: usize) {
        self.focus = Focus::Chat;
        if at < self.chat.rows.len() {
            self.chat.row = at;
            self.chat_open_row();
        }
    }

    /// What a key does in Slack's column: whether it was taken. Those that
    /// act on mail do nothing here.
    pub(super) fn chat_column_act(&mut self, act: Act) -> bool {
        match act {
            Act::Help
            | Act::Calendar
            | Act::NextPane
            | Act::PreviousPane
            | Act::Left
            | Act::Slack
            | Act::Undo
            | Act::Back => return false,
            Act::Down => self.chat_step(1),
            Act::Up => self.chat_step(-1),
            Act::PageDown => self.chat_step(10),
            Act::PageUp => self.chat_step(-10),
            Act::Top => self.chat_step(isize::MIN / 2),
            Act::End => self.chat_step(isize::MAX / 2),
            Act::Open | Act::Right => self.chat_open_row(),
            Act::ScreenDown => match self.chat.rows.get(self.chat.row).copied() {
                Some(ChatRow::Head(s) | ChatRow::More(s)) => self.chat_fold(s),
                _ => self.chat_step(10),
            },
            Act::ScreenUp => self.chat_step(-10),
            Act::Search => self.open_search(),
            _ => self.status = "Slack: ⏎ reads one, K hides the column".to_string(),
        }
        true
    }

    /// Every conversation of `space` shown, or a few again.
    pub(super) fn chat_fold(&mut self, space: usize) {
        if let Some(s) = self.chat.spaces.get_mut(space) {
            s.unfolded = !s.unfolded;
            self.status = if s.unfolded {
                format!("every conversation of {} shown", s.name)
            } else {
                format!("the busiest conversations of {} shown", s.name)
            };
        }
        self.chat_rebuild();
    }

    /// Read the conversation `i` of `space`, in the list's place.
    pub(super) fn open_talk(&mut self, space: usize, i: usize) {
        let Some(talk) = self.chat.spaces.get(space).and_then(|s| s.talks.get(i)) else {
            return;
        };
        let id = talk.channel.id.clone();
        let name = format!("{}{}", talk.channel.kind.mark(), talk.channel.name);
        self.open_id(space, id, name);
    }

    /// Read the conversation `id` of `space`, called `name`, in the list's
    /// place: one of yours, or one a search found.
    fn open_id(&mut self, space: usize, id: String, name: String) {
        if self.chat.open.as_ref().is_some_and(|o| o.id == id) {
            return;
        }
        self.chat.open = Some(Open {
            space,
            id: id.clone(),
            name,
            messages: Vec::new(),
            loading: true,
            error: None,
            picked: None,
            thread: None,
            asked: Instant::now(),
            hits: None,
            pick_ts: None,
        });
        self.chat.ask(space, SlackAsk::History(id));
    }

    /// `/` in Slack: the box its search is typed in.
    pub(super) fn open_search(&mut self) {
        if self.chat.spaces.is_empty() {
            self.status = "no Slack workspace yet: letterbox slack add".to_string();
            return;
        }
        self.chat.search = Some(String::new());
    }

    /// A key while a search is typed: ⏎ asks Slack, and lists what it finds
    /// in the list's place.
    pub(super) fn search_key(&mut self, key: KeyEvent) {
        let ctrl = key.modifiers.contains(KeyModifiers::CONTROL);
        let Some(words) = &mut self.chat.search else {
            return;
        };
        match key.code {
            KeyCode::Esc => self.chat.search = None,
            KeyCode::Char('g') if ctrl => self.chat.search = None,
            KeyCode::Enter if words.trim().is_empty() => {}
            KeyCode::Enter => {
                let words = words.trim().to_string();
                self.chat.search = None;
                let space = self.chat.open.as_ref().map_or(0, |o| o.space);
                self.chat.open = Some(Open {
                    space,
                    id: String::new(),
                    name: format!("Found: {words}"),
                    messages: Vec::new(),
                    loading: true,
                    error: None,
                    picked: None,
                    thread: None,
                    asked: Instant::now(),
                    hits: Some(Vec::new()),
                    pick_ts: None,
                });
                self.focus = Focus::List;
                self.chat.ask(space, SlackAsk::Search(words));
            }
            _ => {
                edit_line(words, key);
            }
        }
    }

    /// The message a search found, picked, gone to: its conversation read,
    /// the message picked once it is, and its thread, if it is a reply.
    fn go_to_hit(&mut self) {
        let Some(open) = &self.chat.open else {
            return;
        };
        let Some(hit) = open
            .picked
            .and_then(|i| open.hits.as_ref()?.get(i))
            .cloned()
        else {
            return;
        };
        let space = open.space;
        self.chat.open = None;
        match self.chat.spaces[space]
            .talks
            .iter()
            .position(|t| t.channel.id == hit.channel)
        {
            Some(i) => self.read_talk(space, i),
            None => self.open_id(space, hit.channel.clone(), hit.place.clone()),
        }
        if let Some(open) = &mut self.chat.open {
            open.pick_ts = Some(hit.message.ts.clone());
            if let Some(ts) = hit.thread.clone() {
                open.thread = Some(OpenThread {
                    ts: ts.clone(),
                    messages: Vec::new(),
                    loading: true,
                    error: None,
                    scroll: 0,
                });
                self.chat.ask(space, SlackAsk::Thread(hit.channel, ts));
            }
        }
        self.focus = Focus::List;
    }

    /// Mail listed again: the conversation read let go.
    pub(super) fn close_talk(&mut self) {
        self.chat.open = None;
    }

    /// What a key does while a conversation is read, out of the sidebar:
    /// whether it was taken. Those that would act on mail are taken too,
    /// and do nothing.
    pub(super) fn talk_act(&mut self, act: Act) -> bool {
        if matches!(self.focus, Focus::Side | Focus::Chat) || self.chat.open.is_none() {
            return false;
        }
        let reader = self.focus == Focus::Reader;
        match act {
            Act::Help
            | Act::Calendar
            | Act::NextPane
            | Act::PreviousPane
            | Act::Left
            | Act::Undo => return false,
            Act::Back if reader => self.focus = Focus::List,
            Act::Back => {}
            Act::Down if reader => self.thread_scroll(1),
            Act::Up if reader => self.thread_scroll(-1),
            Act::PageDown | Act::ScreenDown if reader => self.thread_scroll(10),
            Act::PageUp | Act::ScreenUp if reader => self.thread_scroll(-10),
            Act::Down => self.talk_pick(1),
            Act::Up => self.talk_pick(-1),
            Act::PageDown | Act::ScreenDown => self.talk_pick(10),
            Act::PageUp | Act::ScreenUp => self.talk_pick(-10),
            Act::Top if !reader => self.talk_pick(isize::MIN / 2),
            Act::End if !reader => self.talk_pick(isize::MAX / 2),
            Act::Search => self.open_search(),
            Act::Open | Act::Right
                if !reader && self.chat.open.as_ref().is_some_and(|o| o.hits.is_some()) =>
            {
                self.go_to_hit()
            }
            Act::Open | Act::Right if !reader => self.open_thread(),
            Act::NewMail => self.read_talk_again(),
            Act::Reply if self.chat.open.as_ref().is_some_and(|o| o.hits.is_some()) => {
                self.status = "what was found: ⏎ goes to a message, to answer it there".to_string()
            }
            Act::Reply => self.write_talk(reader),
            Act::Save | Act::SaveAll if !reader => self.save_talk_files(),
            Act::ReplyAll | Act::Forward => {
                self.status =
                    "in Slack, r writes: to the conversation, or in the thread read".to_string();
            }
            _ => {
                self.status = "in Slack: tab or ← to the sidebar, then a mail folder".to_string();
            }
        }
        true
    }

    fn talk_pick(&mut self, by: isize) {
        let Some(open) = &mut self.chat.open else {
            return;
        };
        if open.messages.is_empty() {
            return;
        }
        let last = open.messages.len() - 1;
        let at = open.picked.unwrap_or(last);
        open.picked = Some(at.saturating_add_signed(by).min(last));
    }

    fn thread_scroll(&mut self, by: isize) {
        if let Some(thread) = self.chat.open.as_mut().and_then(|o| o.thread.as_mut()) {
            thread.scroll = thread.scroll.saturating_add_signed(by);
        }
    }

    /// The thread of the message picked, read in the message's place.
    fn open_thread(&mut self) {
        let Some(open) = &mut self.chat.open else {
            return;
        };
        let Some(message) = open.picked.and_then(|i| open.messages.get(i)) else {
            return;
        };
        let ts = message.ts.clone();
        open.thread = Some(OpenThread {
            ts: ts.clone(),
            messages: Vec::new(),
            loading: true,
            error: None,
            scroll: 0,
        });
        let (space, id) = (open.space, open.id.clone());
        self.chat.ask(space, SlackAsk::Thread(id, ts));
        self.focus = Focus::Reader;
    }

    /// Write to the conversation read; or, from the thread read, in it.
    fn write_talk(&mut self, in_thread: bool) {
        let Some(open) = &self.chat.open else {
            return;
        };
        let thread = in_thread
            .then(|| open.thread.as_ref().map(|t| t.ts.clone()))
            .flatten();
        self.chat.writing = Some(Writing {
            space: open.space,
            channel: open.id.clone(),
            name: open.name.clone(),
            thread,
            buffer: crate::edit::Buffer::from_text(""),
            prefix: None,
            sending: false,
            error: None,
        });
        self.status = "C-⏎ sends it (M-⏎ or C-c C-c too) · esc lets it go".to_string();
    }

    /// A key while a message is written: to it, or sending it.
    pub(super) fn chat_write_key(&mut self, key: KeyEvent) {
        let Some(writing) = &mut self.chat.writing else {
            return;
        };
        if writing.sending {
            self.status = "sending…".to_string();
            return;
        }
        // Ctrl+Enter, as chat apps send with, where the terminal tells it from
        // Enter; Alt+Enter, which every terminal does; and C-c C-c, as mail's.
        let send_key = key.code == KeyCode::Enter
            && key
                .modifiers
                .intersects(KeyModifiers::CONTROL | KeyModifiers::ALT)
            && writing.prefix.is_none();
        let done = if send_key {
            Done::Send
        } else {
            crate::edit::handle(&mut writing.buffer, &mut writing.prefix, key)
        };
        match done {
            Done::Send if writing.buffer.text().trim().is_empty() => {
                self.status = "nothing written to send".to_string();
            }
            Done::Send => {
                writing.sending = true;
                writing.error = None;
                let ask = SlackAsk::Post {
                    channel: writing.channel.clone(),
                    thread: writing.thread.clone(),
                    // The line the buffer ends with is not sent.
                    text: writing.buffer.text().trim_end().to_string(),
                };
                let space = writing.space;
                self.status = format!("sending to {}…", writing.name);
                self.chat.ask(space, ask);
            }
            Done::Cancel => {
                self.chat.writing = None;
                self.status = "let go, not sent".to_string();
            }
            _ => {}
        }
    }

    /// The files of the message picked, fetched and saved where mail's are,
    /// each under a name not yet taken there.
    fn save_talk_files(&mut self) {
        let Some(open) = &self.chat.open else {
            return;
        };
        let Some(message) = open.picked.and_then(|i| open.messages.get(i)) else {
            return;
        };
        if message.files.is_empty() {
            self.status = "no files with that message".to_string();
            return;
        }
        let space = open.space;
        let asks: Vec<SlackAsk> = message
            .files
            .iter()
            .map(|f| SlackAsk::Download {
                url: f.url.clone(),
                name: f.name.clone(),
            })
            .collect();
        self.status = format!("fetching {} file(s) from Slack…", asks.len());
        for ask in asks {
            self.chat.ask(space, ask);
        }
    }

    /// What is pasted, into the message being written.
    pub(super) fn chat_paste(&mut self, text: &str) -> bool {
        match &mut self.chat.writing {
            Some(w) if !w.sending => {
                w.buffer.paste(text);
                true
            }
            Some(_) => true,
            None => false,
        }
    }

    /// The conversation read, and its thread, read again; and the counts.
    fn read_talk_again(&mut self) {
        let Some(space) = self.read_open_again(|_| true) else {
            return;
        };
        self.chat.ask(space, SlackAsk::Conversations);
        self.status = "reading Slack again".to_string();
    }

    /// The conversation read, read again; and its thread too, if
    /// `thread_too` says so of the message it began with. Its workspace.
    fn read_open_again(&mut self, thread_too: impl Fn(&str) -> bool) -> Option<usize> {
        // What a search found is not a conversation to read again.
        let open = self.chat.open.as_mut().filter(|o| o.hits.is_none())?;
        open.asked = Instant::now();
        let (space, id) = (open.space, open.id.clone());
        let thread = open
            .thread
            .as_ref()
            .map(|t| t.ts.clone())
            .filter(|ts| thread_too(ts));
        self.chat.ask(space, SlackAsk::History(id.clone()));
        if let Some(ts) = thread {
            self.chat.ask(space, SlackAsk::Thread(id, ts));
        }
        Some(space)
    }

    /// What Slack's threads have answered; whether anything was.
    pub(super) fn chat_pump(&mut self) -> bool {
        let mut any = false;
        while let Ok(reply) = self.chat.replies.try_recv() {
            self.chat_heard(reply);
            any = true;
        }
        any
    }

    fn chat_heard(&mut self, reply: slack_worker::Reply) {
        self.chat.pending = self.chat.pending.saturating_sub(match reply.answer {
            Heard::Unread(..) => 0,
            _ => 1,
        });
        let Some(space) = self.chat.spaces.get_mut(reply.space) else {
            return;
        };
        match reply.answer {
            Heard::Conversations(Ok(found)) => {
                let before: HashMap<String, Option<u32>> = space
                    .talks
                    .drain(..)
                    .map(|t| (t.channel.id, t.unread))
                    .collect();
                space.talks = found
                    .into_iter()
                    .map(|channel| Talk {
                        unread: before.get(&channel.id).copied().flatten(),
                        channel,
                    })
                    .collect();
                space.loading = false;
                space.error = None;
                self.chat_rebuild();
            }
            Heard::Conversations(Err(why)) => {
                space.loading = false;
                space.error = Some(why);
                self.chat_rebuild();
            }
            Heard::Unread(id, n) => {
                if let Some(talk) = space.talks.iter_mut().find(|t| t.channel.id == id) {
                    let shown_before = talk.unread.is_some_and(|u| u > 0);
                    talk.unread = Some(n);
                    if shown_before != (n > 0) {
                        self.chat_rebuild();
                    }
                }
            }
            Heard::Counted => {}
            Heard::Searched(words, found) => {
                let Some(open) = self
                    .chat
                    .open
                    .as_mut()
                    .filter(|o| o.hits.is_some() && o.name == format!("Found: {words}"))
                else {
                    return;
                };
                open.loading = false;
                match found {
                    Ok(hits) => {
                        open.messages = hits
                            .iter()
                            .map(|h| crate::slack::reading::Message {
                                who: format!("{} · {}", h.message.who, h.place),
                                ..h.message.clone()
                            })
                            .collect();
                        open.picked = (!hits.is_empty()).then_some(0);
                        open.hits = Some(hits);
                        open.error = None;
                    }
                    Err(why) => open.error = Some(why),
                }
            }
            Heard::Downloaded(name, got) => {
                let dir = self.save_dir.clone().unwrap_or_else(files::downloads);
                self.status = match got.and_then(|bytes| {
                    saving::save_one(&dir, &name, &bytes).map_err(|e| e.to_string())
                }) {
                    Ok(path) => format!("saved {}", tilde(&path)),
                    Err(why) => format!("{name} not saved: {why}"),
                };
            }
            Heard::Posted(channel, thread, sent) => {
                let Some(writing) = self
                    .chat
                    .writing
                    .as_mut()
                    .filter(|w| w.channel == channel && w.thread == thread && w.sending)
                else {
                    return;
                };
                writing.sending = false;
                match sent {
                    Ok(_) => {
                        let name = writing.name.clone();
                        self.chat.writing = None;
                        self.status = format!("sent to {name}");
                        if self.chat.open.as_ref().is_some_and(|o| o.id == channel) {
                            self.read_open_again(|ts| thread.as_deref() == Some(ts));
                        }
                    }
                    Err(why) => {
                        self.status = format!("not sent: {why}");
                        writing.error = Some(why);
                    }
                }
            }
            Heard::Live(crate::slack::live::Event::Connected, _) => space.live = true,
            Heard::Live(crate::slack::live::Event::Lost(_), _) => space.live = false,
            Heard::Live(crate::slack::live::Event::Message(m), mine) => {
                let read_here = self
                    .chat
                    .open
                    .as_ref()
                    .is_some_and(|o| o.space == reply.space && o.id == m.channel);
                if read_here {
                    self.read_open_again(|ts| m.thread.as_deref() == Some(ts) || m.ts == ts);
                } else if m.new && !mine && m.thread.is_none() {
                    let Some(talk) = self.chat.spaces[reply.space]
                        .talks
                        .iter_mut()
                        .find(|t| t.channel.id == m.channel)
                    else {
                        return;
                    };
                    let was = talk.unread.unwrap_or(0);
                    talk.unread = Some(was + 1);
                    if was == 0 {
                        self.chat_rebuild();
                    }
                }
            }
            Heard::History(id, read) => {
                let Some(open) = self.chat.open.as_mut().filter(|o| o.id == id) else {
                    return;
                };
                open.loading = false;
                match read {
                    Ok(messages) => {
                        // At the newest, or nowhere yet: the newest, still.
                        let newest = open.picked.is_none_or(|p| p + 1 >= open.messages.len());
                        let picked_ts = open
                            .picked
                            .and_then(|p| open.messages.get(p))
                            .map(|m| m.ts.clone());
                        open.messages = messages;
                        let gone_to = open
                            .pick_ts
                            .take()
                            .and_then(|ts| open.messages.iter().position(|m| m.ts == ts));
                        open.picked = if open.messages.is_empty() {
                            None
                        } else if gone_to.is_some() {
                            gone_to
                        } else if newest {
                            Some(open.messages.len() - 1)
                        } else {
                            picked_ts
                                .and_then(|ts| open.messages.iter().position(|m| m.ts == ts))
                                .or(Some(open.messages.len() - 1))
                        };
                        open.error = None;
                    }
                    Err(why) => open.error = Some(why),
                }
            }
            Heard::Thread(id, ts, read) => {
                let Some(thread) = self
                    .chat
                    .open
                    .as_mut()
                    .filter(|o| o.id == id)
                    .and_then(|o| o.thread.as_mut())
                    .filter(|t| t.ts == ts)
                else {
                    return;
                };
                thread.loading = false;
                match read {
                    Ok(messages) => {
                        thread.messages = messages;
                        thread.error = None;
                    }
                    Err(why) => thread.error = Some(why),
                }
            }
        }
    }

    /// Slack read again where it is due; whether anything was asked.
    /// Heard as things happen, the conversation read is read again as a
    /// message comes in it, and counts go up as messages come elsewhere: it is
    /// not read again on a clock, and the counts are made anew less often.
    pub(super) fn chat_tick(&mut self) -> bool {
        let mut any = false;
        let live = |chat: &Chat, space: usize| chat.spaces.get(space).is_some_and(|s| s.live);
        if let Some(open) = &self.chat.open
            && open.hits.is_none()
            && !live(&self.chat, open.space)
            && open.asked.elapsed() >= READ_AGAIN
        {
            self.read_open_again(|_| false);
            any = true;
        }
        for space in 0..self.chat.spaces.len() {
            let again = if live(&self.chat, space) {
                COUNT_AGAIN_LIVE
            } else {
                COUNT_AGAIN
            };
            if self.chat.spaces[space].asked.elapsed() >= again {
                self.chat.ask(space, SlackAsk::Conversations);
                any = true;
            }
        }
        any
    }

    /// Take `answer` as if Slack's threads had given it: for tests.
    #[cfg(test)]
    pub fn chat_hear(&mut self, space: usize, answer: slack_worker::Answer) {
        self.chat.pending += 1;
        self.chat_heard(slack_worker::Reply { space, answer });
    }

    /// Wait until Slack's threads have answered all that was asked, or until
    /// `limit`: for tests.
    #[cfg(test)]
    pub fn settle_chat(&mut self, limit: Duration) {
        let until = Instant::now() + limit;
        while self.chat.pending > 0 {
            let left = until.saturating_duration_since(Instant::now());
            match self.chat.replies.recv_timeout(left) {
                Ok(reply) => self.chat_heard(reply),
                Err(_) => return,
            }
        }
    }
}
