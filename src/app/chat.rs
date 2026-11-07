//! Slack beside the mail: each workspace's conversations in the sidebar, the
//! busiest first, a few shown and the rest folded but for those with anything
//! unread; one read in the list's place, and a thread in the message's.
//! Reading marks nothing read in Slack. What is written, `r`, goes only once
//! it is shown whole, with where it goes, and y is pressed: never on its own.
//! While a conversation is read, the keys that act on mail do nothing: the
//! mail behind it is out of sight.
use super::*;
use crate::slack::Slack;
use crate::slack::reading::Message as Said;
use crate::slack::worker::{self as slack_worker, Answer as Heard, Ask as SlackAsk};

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
}

pub struct OpenThread {
    pub ts: String,
    pub messages: Vec<Said>,
    pub loading: bool,
    pub error: Option<String>,
    pub scroll: usize,
}

/// A message being written: to which conversation, in which thread, if a
/// reply; what is written; whether it is asked to go, and whether it is on
/// its way; and why it did not go, if it did not.
pub struct Writing {
    pub space: usize,
    pub channel: String,
    pub name: String,
    pub thread: Option<String>,
    pub buffer: crate::edit::Buffer,
    prefix: Option<char>,
    pub asking: bool,
    pub sending: bool,
    pub error: Option<String>,
}

pub struct Chat {
    pub spaces: Vec<Space>,
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
        self.rebuild();
    }

    /// The sidebar's rows of Slack, each workspace's.
    pub(super) fn chat_rows(&self) -> Vec<Row> {
        let mut rows = Vec::new();
        for (s, space) in self.chat.spaces.iter().enumerate() {
            rows.push(Row::SlackHead(s));
            let shown = self.chat.shown(s);
            rows.extend(shown.iter().map(|&i| Row::Talk(s, i)));
            if space.unfolded || shown.len() < space.talks.len() {
                rows.push(Row::TalkMore(s));
            }
        }
        rows
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
        self.rebuild();
    }

    /// Read the conversation `i` of `space`, in the list's place.
    pub(super) fn open_talk(&mut self, space: usize, i: usize) {
        let Some(talk) = self.chat.spaces.get(space).and_then(|s| s.talks.get(i)) else {
            return;
        };
        let id = talk.channel.id.clone();
        let name = format!("{}{}", talk.channel.kind.mark(), talk.channel.name);
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
        });
        self.chat.ask(space, SlackAsk::History(id));
    }

    /// Mail listed again: the conversation read let go.
    pub(super) fn close_talk(&mut self) {
        self.chat.open = None;
    }

    /// What a key does while a conversation is read, out of the sidebar:
    /// whether it was taken. Those that would act on mail are taken too,
    /// and do nothing.
    pub(super) fn talk_act(&mut self, act: Act) -> bool {
        if self.focus == Focus::Side || self.chat.open.is_none() {
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
            Act::Open | Act::Right if !reader => self.open_thread(),
            Act::NewMail => self.read_talk_again(),
            Act::Reply => self.write_talk(reader),
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
            asking: false,
            sending: false,
            error: None,
        });
        self.status = "C-c C-c asks to send it · esc lets it go".to_string();
    }

    /// A key while a message is written: to it; or, asked whether it goes,
    /// y sends it, n goes back to it. Nothing else sends.
    pub(super) fn chat_write_key(&mut self, key: KeyEvent) {
        let Some(writing) = &mut self.chat.writing else {
            return;
        };
        if writing.sending {
            self.status = "sending…".to_string();
            return;
        }
        let plain = !key
            .modifiers
            .intersects(KeyModifiers::CONTROL | KeyModifiers::ALT);
        if writing.asking {
            match key.code {
                KeyCode::Char('y' | 'Y') if plain => {
                    writing.asking = false;
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
                KeyCode::Char('n' | 'N') if plain => {
                    writing.asking = false;
                    self.status = "not sent · C-c C-c asks again, esc lets it go".to_string();
                }
                KeyCode::Esc => {
                    writing.asking = false;
                    self.status = "not sent".to_string();
                }
                _ => self.status = "y sends it, n goes back to it".to_string(),
            }
            return;
        }
        match crate::edit::handle(&mut writing.buffer, &mut writing.prefix, key) {
            Done::Send if writing.buffer.text().trim().is_empty() => {
                self.status = "nothing written to send".to_string();
            }
            Done::Send => {
                writing.asking = true;
                self.status = format!(
                    "send this to {} as you? y sends it, n goes back to it",
                    writing.name
                );
            }
            Done::Cancel => {
                self.chat.writing = None;
                self.status = "let go, not sent".to_string();
            }
            _ => {}
        }
    }

    /// What is pasted, into the message being written.
    pub(super) fn chat_paste(&mut self, text: &str) -> bool {
        match &mut self.chat.writing {
            Some(w) if !w.asking && !w.sending => {
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
        let open = self.chat.open.as_mut()?;
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
                self.rebuild();
            }
            Heard::Conversations(Err(why)) => {
                space.loading = false;
                space.error = Some(why);
                self.rebuild();
            }
            Heard::Unread(id, n) => {
                if let Some(talk) = space.talks.iter_mut().find(|t| t.channel.id == id) {
                    let shown_before = talk.unread.is_some_and(|u| u > 0);
                    talk.unread = Some(n);
                    if shown_before != (n > 0) {
                        self.rebuild();
                    }
                }
            }
            Heard::Counted => {}
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
                        self.rebuild();
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
                        open.picked = if open.messages.is_empty() {
                            None
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
