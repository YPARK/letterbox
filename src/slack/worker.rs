//! A workspace asked from threads of its own, so the screen never waits on
//! Slack: one for what is read, one that counts what is unread, slowly, as
//! that takes a call or two a conversation and Slack limits how many.

use super::reading::{Message, People, named};
use super::{Channel, Slack};
use std::sync::mpsc::{self, Receiver, Sender};

pub enum Ask {
    /// The conversations this workspace has of yours, and what each has unread.
    Conversations,
    History(String),
    /// A thread, by its conversation and the message it began with.
    Thread(String, String),
    /// Send, as you, what was written and said yes to: to a conversation,
    /// in a thread if one is given.
    Post {
        channel: String,
        thread: Option<String>,
        text: String,
    },
}

pub enum Answer {
    /// Your conversations, the busiest first, each direct one under a name.
    Conversations(Result<Vec<Channel>, String>),
    Unread(String, u32),
    /// Every conversation asked has been counted.
    Counted,
    History(String, Result<Vec<Message>, String>),
    Thread(String, String, Result<Vec<Message>, String>),
    /// What was sent, to where, or why it was not.
    Posted(String, Option<String>, Result<String, String>),
    /// What happened, as it happened, and whether it was yours.
    Live(super::live::Event, bool),
}

pub struct Reply {
    pub space: usize,
    pub answer: Answer,
}

fn said(e: anyhow::Error) -> String {
    format!("{e:#}")
}

/// Start asking `slack` for the workspace `space`, answering on `replies`;
/// and hearing what happens as it does, by the app's token, if there is one.
pub fn spawn(
    space: usize,
    slack: Slack,
    app: Option<String>,
    replies: Sender<Reply>,
) -> Sender<Ask> {
    if let Some(token) = app {
        let (slack, replies) = (slack.clone(), replies.clone());
        std::thread::spawn(move || hears(space, slack, &token, replies));
    }
    let (asks, asked) = mpsc::channel();
    let (count, counting) = mpsc::channel::<Vec<Channel>>();
    let counter = (slack.clone(), replies.clone());
    std::thread::spawn(move || counts(space, counter.0, counting, counter.1));
    std::thread::spawn(move || reads(space, slack, asked, count, replies));
    asks
}

fn reads(
    space: usize,
    slack: Slack,
    asked: Receiver<Ask>,
    count: Sender<Vec<Channel>>,
    replies: Sender<Reply>,
) {
    let mut people: Option<People> = None;
    while let Ok(ask) = asked.recv() {
        if people.is_none() {
            people = slack.people().ok();
        }
        let known = people.clone().unwrap_or_default();
        let answer = match ask {
            Ask::Conversations => {
                let found = slack.mine().map(|mine| {
                    let mut mine = named(mine, &known);
                    mine.sort_by_key(|c| std::cmp::Reverse(c.updated));
                    mine
                });
                match &found {
                    Ok(mine) => {
                        let _ = count.send(mine.clone());
                    }
                    // Nothing to count: said counted, all the same.
                    Err(_) => {
                        let _ = replies.send(Reply {
                            space,
                            answer: Answer::Counted,
                        });
                    }
                }
                Answer::Conversations(found.map_err(said))
            }
            Ask::History(id) => {
                let read = slack.messages(&id, HISTORY, &known).map_err(said);
                Answer::History(id, read)
            }
            Ask::Thread(id, ts) => {
                let read = slack.thread(&id, &ts, &known).map_err(said);
                Answer::Thread(id, ts, read)
            }
            Ask::Post {
                channel,
                thread,
                text,
            } => {
                let sent = slack.post(&channel, &text, thread.as_deref()).map_err(said);
                Answer::Posted(channel, thread, sent)
            }
        };
        if replies.send(Reply { space, answer }).is_err() {
            return;
        }
    }
}

fn hears(space: usize, slack: Slack, token: &str, replies: Sender<Reply>) {
    let me = slack.whoami().map(|w| w.user_id).unwrap_or_default();
    super::live::Socket::new(token).listen(&mut |event| {
        let mine =
            matches!(&event, super::live::Event::Message(m) if !me.is_empty() && m.user == me);
        let answer = Answer::Live(event, mine);
        replies.send(Reply { space, answer }).is_ok()
    });
}

/// How many of a conversation's last messages are read.
pub const HISTORY: usize = 100;

fn counts(space: usize, slack: Slack, counting: Receiver<Vec<Channel>>, replies: Sender<Reply>) {
    while let Ok(mut channels) = counting.recv() {
        // Only the latest asked is counted: those before it are older.
        while let Ok(later) = counting.try_recv() {
            channels = later;
            let _ = replies.send(Reply {
                space,
                answer: Answer::Counted,
            });
        }
        for channel in channels {
            if let Ok(n) = slack.unread(&channel) {
                let answer = Answer::Unread(channel.id.clone(), n);
                if replies.send(Reply { space, answer }).is_err() {
                    return;
                }
            }
        }
        if replies
            .send(Reply {
                space,
                answer: Answer::Counted,
            })
            .is_err()
        {
            return;
        }
    }
}
