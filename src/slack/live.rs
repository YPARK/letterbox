//! Slack's events as they happen, over Socket Mode: Slack hands out a socket
//! to the app-level token of your app, and sends on it what happens in the
//! conversations you are in, each answered as it asks. Only which
//! conversation, and which message, is taken: what it says is read again
//! from the Web API, as the rest is.

use crate::imap::wire::{self, Stream};
use crate::net;
use anyhow::{Context, anyhow, bail};
use serde_json::{Value, json};
use std::time::Duration;
use tungstenite::{Message, WebSocket};

const API: &str = "https://slack.com/api";
/// How long it is waited before connecting again, at first, and at most.
const FIRST_WAIT: Duration = Duration::from_millis(if cfg!(test) { 10 } else { 1000 });
const LONGEST_WAIT: Duration = Duration::from_secs(60);

/// A message come, changed, or gone: its conversation, its ID, who wrote it,
/// and the thread it is in, if it is a reply.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Live {
    pub channel: String,
    pub ts: String,
    pub user: String,
    pub thread: Option<String>,
    pub new: bool,
}

impl Live {
    fn read(event: &Value) -> Option<Live> {
        if event["type"] != "message" {
            return None;
        }
        // An edit says the message as it is now; a deletion, as it was.
        let inner = [&event["message"], &event["previous_message"]]
            .into_iter()
            .find(|m| m.is_object())
            .unwrap_or(event);
        let text = |v: &Value, name: &str| v[name].as_str().map(str::to_string);
        let ts = text(inner, "ts").or_else(|| text(event, "ts"))?;
        let thread = text(inner, "thread_ts").filter(|t| *t != ts);
        // Said anew, not an edit, a deletion, or someone joining.
        let new = std::ptr::eq(inner, event)
            && matches!(
                event["subtype"].as_str(),
                None | Some("thread_broadcast" | "file_share" | "bot_message" | "me_message")
            );
        Some(Live {
            channel: text(event, "channel")?,
            user: text(inner, "user")
                .or_else(|| text(event, "user"))
                .unwrap_or_default(),
            ts,
            thread,
            new,
        })
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Event {
    /// The socket is open, and events come.
    Connected,
    Message(Live),
    /// The socket was lost, and why; it is opened again after a while.
    Lost(String),
}

/// What became of a socket: to be let go, or opened again at once.
enum Then {
    Stop,
    Again,
}

/// Slack's socket, by an app-level token.
pub struct Socket {
    base: String,
    token: String,
    agent: ureq::Agent,
}

impl Socket {
    pub fn new(token: &str) -> Socket {
        Socket::at(API, token)
    }

    /// Slack, at `base`: elsewhere only for a test.
    pub(crate) fn at(base: &str, token: &str) -> Socket {
        Socket {
            base: base.to_string(),
            token: token.to_string(),
            agent: net::agent(),
        }
    }

    /// Where to open a socket, as Slack hands one out.
    pub fn link(&self) -> anyhow::Result<String> {
        let mut answer = self
            .agent
            .post(format!("{}/apps.connections.open", self.base))
            .header("Authorization", format!("Bearer {}", self.token))
            .send_empty()
            .context("cannot reach Slack for its socket")?;
        let text = answer
            .body_mut()
            .read_to_string()
            .context("cannot read Slack's answer for its socket")?;
        let v: Value =
            serde_json::from_str(&text).context("cannot read Slack's answer for its socket")?;
        if v["ok"].as_bool() != Some(true) {
            let error = v["error"].as_str().unwrap_or("no reason given");
            bail!(
                "Slack refused its socket: {error}. Is the app-level token right, and Socket Mode on?"
            );
        }
        v["url"]
            .as_str()
            .map(str::to_string)
            .context("Slack gave no socket")
    }

    /// Hear what happens, each event given to `on`, until it says to stop;
    /// a socket lost, or ended by Slack, opened again.
    pub fn listen(&self, on: &mut dyn FnMut(Event) -> bool) {
        let mut wait = FIRST_WAIT;
        loop {
            match self.session(on) {
                Ok(Then::Stop) => return,
                Ok(Then::Again) => wait = FIRST_WAIT,
                Err(e) => {
                    if !on(Event::Lost(format!("{e:#}"))) {
                        return;
                    }
                    std::thread::sleep(wait);
                    wait = (wait * 2).min(LONGEST_WAIT);
                }
            }
        }
    }

    fn session(&self, on: &mut dyn FnMut(Event) -> bool) -> anyhow::Result<Then> {
        let mut ws = open(&self.link()?)?;
        // Quiet once, a ping asks whether it is still there; quiet twice, gone.
        let mut quiet = false;
        loop {
            let frame = match ws.read() {
                Ok(frame) => frame,
                Err(tungstenite::Error::Io(e))
                    if !quiet
                        && matches!(
                            e.kind(),
                            std::io::ErrorKind::WouldBlock | std::io::ErrorKind::TimedOut
                        ) =>
                {
                    quiet = true;
                    ws.send(Message::Ping(Vec::new().into()))?;
                    continue;
                }
                Err(e) => return Err(e).context("Slack's socket was lost"),
            };
            quiet = false;
            let text = match frame {
                Message::Text(text) => text,
                Message::Close(_) => return Ok(Then::Again),
                _ => continue,
            };
            let Ok(v) = serde_json::from_str::<Value>(&text) else {
                continue;
            };
            if let Some(id) = v["envelope_id"].as_str() {
                ws.send(json!({"envelope_id": id}).to_string().into())?;
            }
            let event = match v["type"].as_str() {
                Some("hello") => Event::Connected,
                Some("disconnect") => return Ok(Then::Again),
                Some("events_api") => match Live::read(&v["payload"]["event"]) {
                    Some(live) => Event::Message(live),
                    None => continue,
                },
                _ => continue,
            };
            if !on(event) {
                return Ok(Then::Stop);
            }
        }
    }
}

/// A socket open at `url`: over TLS for `wss`, as Slack's are.
fn open(url: &str) -> anyhow::Result<WebSocket<Stream>> {
    let parsed = url::Url::parse(url).with_context(|| "Slack gave a socket not to be read")?;
    let host = parsed
        .host_str()
        .context("Slack gave a socket with no host")?;
    let secure = match parsed.scheme() {
        "wss" => true,
        "ws" => false,
        other => bail!("Slack gave a socket by {other}"),
    };
    let port = parsed.port().unwrap_or(if secure { 443 } else { 80 });
    let tcp = wire::dial(host, port)?;
    let stream = if secure {
        wire::tls(host, tcp)?
    } else {
        Stream::Plain(tcp)
    };
    let (ws, _) =
        tungstenite::client(url, stream).map_err(|e| anyhow!("cannot open Slack's socket: {e}"))?;
    Ok(ws)
}
