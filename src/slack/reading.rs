//! Reading a conversation: who said what, in plain text. Asking for messages
//! this way does not mark them read in Slack.

use super::{Channel, Kind, Slack};
use anyhow::bail;
use serde::Serialize;
use serde_json::Value;
use std::collections::HashMap;

/// The name each user ID goes by.
pub type People = HashMap<String, String>;

/// One message: its time as Slack writes it, which is also its ID; who wrote
/// it; its text; and how many replies it has.
#[derive(Debug, Clone, Serialize)]
pub struct Message {
    pub ts: String,
    pub who: String,
    pub text: String,
    pub replies: u64,
    pub files: Vec<File>,
}

/// A message found by a search: its conversation, by ID and as shown, the
/// message, and the thread it is a reply in, if it is one.
#[derive(Debug, Clone)]
pub struct Hit {
    pub channel: String,
    pub place: String,
    pub message: Message,
    pub thread: Option<String>,
}

impl Slack {
    /// What Slack's own search finds of `words`, the newest first: each
    /// message with its conversation, by name, and its thread, which only
    /// its link says.
    pub fn search(&self, words: &str, people: &People) -> anyhow::Result<Vec<Hit>> {
        let v = self.call(
            "search.messages",
            &[
                ("query", words),
                ("count", "50"),
                ("sort", "timestamp"),
                ("sort_dir", "desc"),
            ],
        )?;
        let matches = v["messages"]["matches"].as_array().into_iter().flatten();
        Ok(matches
            .filter_map(|m| {
                let channel = &m["channel"];
                let name = channel["name"].as_str().unwrap_or("");
                let place = if channel["is_im"].as_bool().unwrap_or(false) {
                    format!("@{}", people.get(name).map_or(name, String::as_str))
                } else {
                    format!("#{name}")
                };
                let thread = m["permalink"]
                    .as_str()
                    .and_then(|p| url::Url::parse(p).ok())
                    .and_then(|u| {
                        u.query_pairs()
                            .find(|(k, _)| k == "thread_ts")
                            .map(|(_, v)| v.into_owned())
                    });
                Some(Hit {
                    channel: channel["id"].as_str()?.to_string(),
                    place,
                    message: Message::read(m, people)?,
                    thread,
                })
            })
            .collect())
    }
}

/// A file a message came with: its name, its size, and where it is fetched.
#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
pub struct File {
    pub name: String,
    pub size: u64,
    pub url: String,
}

impl Message {
    fn read(v: &Value, people: &People) -> Option<Message> {
        let who = v["user"]
            .as_str()
            .map(|id| people.get(id).map_or(id, String::as_str))
            .or(v["bot_profile"]["name"].as_str())
            .or(v["username"].as_str())
            .unwrap_or("");
        Some(Message {
            ts: v["ts"].as_str()?.to_string(),
            who: who.to_string(),
            text: plain(v["text"].as_str().unwrap_or(""), people),
            replies: v["reply_count"].as_u64().unwrap_or(0),
            files: v["files"]
                .as_array()
                .into_iter()
                .flatten()
                .filter_map(|f| {
                    Some(File {
                        name: f["name"].as_str()?.to_string(),
                        size: f["size"].as_u64().unwrap_or(0),
                        url: f["url_private_download"].as_str()?.to_string(),
                    })
                })
                .collect(),
        })
    }
}

impl Slack {
    /// Everyone in the workspace, by the name they show.
    pub fn people(&self) -> anyhow::Result<People> {
        let members = self.paged("users.list", &[], "members")?;
        let mut all = People::new();
        for member in &members {
            let Some(id) = member["id"].as_str() else {
                continue;
            };
            let shown = |name: &str| member["profile"][name].as_str().filter(|s| !s.is_empty());
            let name = shown("display_name")
                .or(shown("real_name"))
                .or(member["name"].as_str())
                .unwrap_or(id);
            all.insert(id.to_string(), name.to_string());
        }
        Ok(all)
    }

    /// The last `count` messages of a conversation, the oldest first.
    pub fn messages(
        &self,
        channel: &str,
        count: usize,
        people: &People,
    ) -> anyhow::Result<Vec<Message>> {
        self.history(channel, count, &[], people)
    }

    /// Up to `count` messages of a conversation since `oldest`, in seconds
    /// since 1970, the oldest first: those it began with, not replies in
    /// threads begun before.
    pub fn since(
        &self,
        channel: &str,
        oldest: i64,
        count: usize,
        people: &People,
    ) -> anyhow::Result<Vec<Message>> {
        let oldest = oldest.to_string();
        self.history(channel, count, &[("oldest", &oldest)], people)
    }

    /// Up to `count` messages of a conversation from before the message
    /// `ts`, the oldest first.
    pub fn before(
        &self,
        channel: &str,
        ts: &str,
        count: usize,
        people: &People,
    ) -> anyhow::Result<Vec<Message>> {
        let bounds = [("latest", ts), ("inclusive", "false")];
        self.history(channel, count, &bounds, people)
    }

    /// Up to `count` messages of a conversation within `bounds`, the oldest
    /// first.
    fn history(
        &self,
        channel: &str,
        count: usize,
        bounds: &[(&str, &str)],
        people: &People,
    ) -> anyhow::Result<Vec<Message>> {
        let limit = count.to_string();
        let mut form = vec![("channel", channel), ("limit", limit.as_str())];
        form.extend_from_slice(bounds);
        let v = self.call("conversations.history", &form)?;
        let mut messages: Vec<_> = v["messages"]
            .as_array()
            .into_iter()
            .flatten()
            .filter_map(|m| Message::read(m, people))
            .collect();
        messages.reverse();
        Ok(messages)
    }

    /// The conversations this session is in, each by its name, the busiest
    /// first.
    pub fn busiest(&self, people: &People) -> anyhow::Result<Vec<Channel>> {
        let mut mine = named(self.mine()?, people);
        mine.sort_by_key(|c| std::cmp::Reverse(c.updated));
        Ok(mine)
    }

    /// All of the thread begun by the message `ts`, in its order.
    pub fn thread(&self, channel: &str, ts: &str, people: &People) -> anyhow::Result<Vec<Message>> {
        let said = self.paged(
            "conversations.replies",
            &[("channel", channel), ("ts", ts)],
            "messages",
        )?;
        Ok(said
            .iter()
            .filter_map(|m| Message::read(m, people))
            .collect())
    }
}

/// `channels`, with each direct message under the other person's name.
pub fn named(mut channels: Vec<Channel>, people: &People) -> Vec<Channel> {
    for channel in &mut channels {
        if channel.kind == Kind::Direct
            && let Some(name) = people.get(&channel.name)
        {
            channel.name = name.clone();
        }
    }
    channels
}

/// The conversation asked for: by its ID, or by its name or a part of it, in
/// any case, `#lab` or `@Ana` to say which kind. A whole name goes before a
/// part of one; where several fit, they are said, not guessed between.
pub fn find<'a>(channels: &'a [Channel], asked: &str) -> anyhow::Result<&'a Channel> {
    if let Some(channel) = channels.iter().find(|c| c.id == asked) {
        return Ok(channel);
    }
    let bare = asked.trim_start_matches(['#', '@']).to_lowercase();
    if bare.is_empty() {
        bail!("no conversation {asked}: see letterbox slack channels");
    }
    let fitting: Vec<_> = channels
        .iter()
        .filter(|c| match asked.chars().next() {
            Some('@') => c.kind == Kind::Direct,
            Some('#') => c.kind != Kind::Direct,
            _ => true,
        })
        .collect();
    let whole: Vec<_> = fitting
        .iter()
        .filter(|c| c.name.to_lowercase() == bare)
        .copied()
        .collect();
    let found = if whole.is_empty() {
        fitting
            .iter()
            .filter(|c| c.name.to_lowercase().contains(&bare))
            .copied()
            .collect()
    } else {
        whole
    };
    match found[..] {
        [] => bail!("no conversation {asked}: see letterbox slack channels"),
        [channel] => Ok(channel),
        _ => {
            let names: Vec<_> = found
                .iter()
                .take(10)
                .map(|c| format!("{} ({})", c.name, c.id))
                .collect();
            bail!("{asked} could be any of: {}", names.join(", "))
        }
    }
}

/// `text`, as Slack writes it, as it reads: names for IDs, links as words and
/// addresses, no escapes. What strangers wrote can hold no terminal controls.
pub fn plain(text: &str, people: &People) -> String {
    let mut out = String::new();
    let mut rest = text;
    while let Some(start) = rest.find('<') {
        let (before, from) = rest.split_at(start);
        out.push_str(&emoji(&unescape(before)));
        let Some(end) = from.find('>') else {
            rest = from;
            break;
        };
        out.push_str(&markup(&from[1..end], people));
        rest = &from[end + 1..];
    }
    out.push_str(&emoji(&unescape(rest)));
    out.chars()
        .filter(|c| !c.is_control() || matches!(c, '\n' | '\t'))
        .collect()
}

/// What one `<...>` of Slack's says.
fn markup(inner: &str, people: &People) -> String {
    let (target, label) = match inner.split_once('|') {
        Some((target, label)) => (target, Some(unescape(label))),
        None => (inner, None),
    };
    if let Some(id) = target.strip_prefix('@') {
        return format!("@{}", people.get(id).map_or(id, String::as_str));
    }
    if let Some(id) = target.strip_prefix('#') {
        return format!("#{}", label.unwrap_or_else(|| id.to_string()));
    }
    if let Some(what) = target.strip_prefix('!') {
        return label.unwrap_or_else(|| format!("@{what}"));
    }
    let address = unescape(target);
    if let Some(mail) = address.strip_prefix("mailto:") {
        return label.unwrap_or_else(|| mail.to_string());
    }
    match label {
        Some(label) if label != address => format!("{label} ({address})"),
        _ => address,
    }
}

/// `:smiley:` as 😃, by the names GitHub and Slack share; a skin tone after
/// one let go. A name not known, as a workspace's own emoji, stays as written.
fn emoji(text: &str) -> String {
    let named = |name: &str| {
        !name.is_empty()
            && name
                .bytes()
                .all(|b| b.is_ascii_alphanumeric() || matches!(b, b'_' | b'+' | b'-'))
    };
    let mut out = String::new();
    let mut rest = text;
    while let Some(start) = rest.find(':') {
        out.push_str(&rest[..start]);
        let after = &rest[start + 1..];
        let found = after
            .find(':')
            .map(|end| (end, &after[..end]))
            .filter(|(_, name)| named(name))
            .and_then(|(end, name)| Some((end, emojis::get_by_shortcode(name)?)));
        match found {
            Some((end, e)) => {
                out.push_str(e.as_str());
                rest = &after[end + 1..];
                // A skin tone, as Slack writes one after the emoji.
                if let Some(more) = rest.strip_prefix(":skin-tone-")
                    && let Some(close) = more.find(':')
                    && more[..close].bytes().all(|b| b.is_ascii_digit())
                {
                    rest = &more[close + 1..];
                }
            }
            None => {
                out.push(':');
                rest = after;
            }
        }
    }
    out.push_str(rest);
    out
}

fn unescape(text: &str) -> String {
    crate::papers::decode(text)
}
