//! Slack, as you are in it: by the user token of an app of your own, asked
//! over its Web API, in plain HTTPS as the rest of letterbox's web. The token
//! is kept as a password is; the file beside the accounts says only which
//! workspaces.

pub mod commands;
pub mod live;
pub mod reading;
pub mod worker;

use crate::net;
use crate::secret::{self, Keeper};
use anyhow::{Context, bail};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::path::{Path, PathBuf};
use std::time::Duration;

const API: &str = "https://slack.com/api";
/// The "host" a token is kept under in the password store.
pub(crate) const KEPT_AT: &str = "slack.com";
/// How many times one call is made, while Slack says to slow down.
const TRIES: usize = 4;
/// The longest Slack is waited for at a time.
const LONGEST: u64 = 60;
/// The largest file fetched.
const FILE_MOST: u64 = 200 << 20;
/// The most unread messages counted of a channel.
pub const UNREAD_MOST: u32 = 100;

/// What Slack is asked with. The token is as good as a password: never printed.
#[derive(Clone, Serialize, Deserialize)]
pub struct Session {
    pub domain: String,
    pub token: String,
    /// The app's own token, for Socket Mode: what happens heard as it does.
    #[serde(default)]
    pub app: Option<String>,
}

impl std::fmt::Debug for Session {
    fn fmt(&self, f: &mut std::fmt::Formatter) -> std::fmt::Result {
        write!(f, "Session({})", self.domain)
    }
}

impl Session {
    /// Keep it in `keep`.
    /// Both tokens are kept together, so they are asked for together.
    pub fn keep(&self, keep: Keeper) -> anyhow::Result<()> {
        secret::store(keep, KEPT_AT, &self.domain, &serde_json::to_string(self)?)
    }

    /// The session kept for a workspace.
    pub fn kept(workspace: &Workspace) -> anyhow::Result<Session> {
        let kept = secret::find(workspace.keep, KEPT_AT, &workspace.domain)
            .context("add the workspace again: letterbox slack add")?;
        if kept.starts_with('{') {
            let mut session: Session = serde_json::from_str(&kept)
                .context("what is kept of the Slack workspace cannot be read")?;
            session.domain = workspace.domain.clone();
            return Ok(session);
        }
        Ok(Session {
            domain: workspace.domain.clone(),
            token: kept,
            app: None,
        })
    }

    /// Forget what is kept for a workspace.
    pub fn forget(workspace: &Workspace) -> anyhow::Result<()> {
        secret::remove(workspace.keep, KEPT_AT, &workspace.domain)
    }
}

/// `typed`, if it is a user token; what it was is not repeated if it is not.
pub fn user_token(typed: &str) -> anyhow::Result<String> {
    let token = typed.trim();
    if !token.starts_with("xoxp-") {
        bail!("that is not a user token: it starts xoxp-, under your app's OAuth & Permissions");
    }
    Ok(token.to_string())
}

/// A workspace added, by its name in `<name>.slack.com`, and where its token
/// is kept.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Workspace {
    pub domain: String,
    pub keep: Keeper,
}

#[derive(Default, Serialize, Deserialize)]
struct Workspaces {
    #[serde(default, rename = "workspace")]
    all: Vec<Workspace>,
}

fn file(accounts: &Path) -> PathBuf {
    accounts.with_file_name("slack.toml")
}

/// The workspaces added.
pub fn workspaces(accounts: &Path) -> anyhow::Result<Vec<Workspace>> {
    let path = file(accounts);
    match std::fs::read_to_string(&path) {
        Ok(text) => {
            let read: Workspaces =
                toml::from_str(&text).with_context(|| format!("cannot read {}", path.display()))?;
            Ok(read.all)
        }
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(Vec::new()),
        Err(e) => Err(e).with_context(|| format!("cannot read {}", path.display())),
    }
}

fn write(accounts: &Path, all: Vec<Workspace>) -> anyhow::Result<()> {
    let path = file(accounts);
    if let Some(dir) = path.parent() {
        std::fs::create_dir_all(dir)?;
    }
    std::fs::write(&path, toml::to_string(&Workspaces { all })?)
        .with_context(|| format!("cannot write {}", path.display()))
}

/// Remember a workspace, in place of itself if it was already.
pub fn remember(accounts: &Path, workspace: &Workspace) -> anyhow::Result<()> {
    let mut all = workspaces(accounts)?;
    all.retain(|w| w.domain != workspace.domain);
    all.push(workspace.clone());
    write(accounts, all)
}

/// Forget a workspace.
pub fn forget_workspace(accounts: &Path, domain: &str) -> anyhow::Result<()> {
    let mut all = workspaces(accounts)?;
    all.retain(|w| w.domain != domain);
    write(accounts, all)
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum Kind {
    Public,
    Private,
    Direct,
    Group,
}

impl Kind {
    /// The mark a conversation of this kind is shown with, before its name.
    pub fn mark(self) -> &'static str {
        match self {
            Kind::Public => "#",
            Kind::Private => "!",
            Kind::Direct => "@",
            Kind::Group => "&",
        }
    }
}

/// When a message was, in seconds since 1970, from its ID as Slack writes
/// it, `1730000000.000100`.
pub fn seconds(ts: &str) -> Option<i64> {
    ts.split('.').next()?.parse().ok()
}

/// A conversation: a channel, a direct message, or a group of them. A direct
/// message's name is, for now, the other person's user ID.
#[derive(Debug, Clone, Serialize)]
pub struct Channel {
    pub id: String,
    pub name: String,
    pub kind: Kind,
    /// When it was last busy, in milliseconds since 1970, as Slack says.
    pub updated: i64,
}

impl Channel {
    fn read(v: &Value) -> Option<Channel> {
        let flag = |name: &str| v[name].as_bool().unwrap_or(false);
        let kind = if flag("is_im") {
            Kind::Direct
        } else if flag("is_mpim") {
            Kind::Group
        } else if flag("is_private") || flag("is_group") {
            Kind::Private
        } else {
            Kind::Public
        };
        let name = v["name"].as_str().or(v["user"].as_str()).unwrap_or("");
        Some(Channel {
            id: v["id"].as_str()?.to_string(),
            name: name.to_string(),
            kind,
            updated: v["updated"].as_i64().unwrap_or(0),
        })
    }
}

/// Who a session is, in what, and that workspace's name.
#[derive(Debug)]
pub struct Who {
    pub user: String,
    pub user_id: String,
    pub team: String,
    pub domain: String,
}

/// `text` as Slack reads it as typed: its <, > and & are Slack's own, so
/// they go escaped.
fn escaped(text: &str) -> String {
    text.replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
}

/// What Slack refused, said as the scope the app needs to `act`, where the
/// refusal is for a scope it lacks.
fn needs(e: anyhow::Error, act: &str, scope: &str) -> anyhow::Error {
    if format!("{e:#}").contains("missing_scope") {
        anyhow::anyhow!(
            "your app may not {act} yet: add {scope} to its User Token Scopes \
             (OAuth & Permissions), reinstall it, then letterbox slack add"
        )
    } else {
        e
    }
}

/// Whether `url` is Slack's own, over HTTPS.
fn slacks(url: &str) -> bool {
    url::Url::parse(url).is_ok_and(|u| {
        u.scheme() == "https"
            && u.host_str()
                .is_some_and(|h| h == "slack.com" || h.ends_with(".slack.com"))
    })
}

/// `acme` of `https://acme.slack.com/`.
fn domain_of(url: &str) -> String {
    url::Url::parse(url)
        .ok()
        .and_then(|u| {
            u.host_str()
                .map(|h| h.split('.').next().unwrap_or("").to_string())
        })
        .unwrap_or_default()
}

/// Slack, asked with one session.
#[derive(Clone)]
pub struct Slack {
    base: String,
    session: Session,
    agent: ureq::Agent,
}

impl Slack {
    pub fn new(session: Session) -> Slack {
        Slack::at(API, session)
    }

    /// Slack, at `base`: elsewhere only for a test.
    pub(crate) fn at(base: &str, session: Session) -> Slack {
        Slack {
            base: base.to_string(),
            session,
            agent: net::agent(),
        }
    }

    /// Call `method`, waiting as long as Slack says when it says to slow down.
    fn call(&self, method: &str, form: &[(&str, &str)]) -> anyhow::Result<Value> {
        let url = format!("{}/{method}", self.base);
        let mut fields = vec![("token", self.session.token.as_str())];
        fields.extend_from_slice(form);
        for _ in 0..TRIES {
            let mut answer = self
                .agent
                .post(&url)
                .send_form(fields.iter().copied())
                .with_context(|| format!("cannot reach Slack for {method}"))?;
            if answer.status() == 429 {
                let wait = answer
                    .headers()
                    .get("retry-after")
                    .and_then(|v| v.to_str().ok())
                    .and_then(|v| v.trim().parse().ok())
                    .unwrap_or(1u64);
                std::thread::sleep(Duration::from_secs(wait.min(LONGEST)));
                continue;
            }
            if !answer.status().is_success() {
                bail!("Slack gave {} for {method}", answer.status());
            }
            let text = answer
                .body_mut()
                .read_to_string()
                .with_context(|| format!("cannot read Slack's answer to {method}"))?;
            let v: Value = serde_json::from_str(&text)
                .with_context(|| format!("cannot read Slack's answer to {method}"))?;
            if v["ok"].as_bool() != Some(true) {
                let error = v["error"].as_str().unwrap_or("no reason given");
                if matches!(
                    error,
                    "invalid_auth" | "not_authed" | "token_revoked" | "token_expired"
                ) {
                    bail!("Slack refused {method}: {error}. Add it again: letterbox slack add");
                }
                bail!("Slack refused {method}: {error}");
            }
            return Ok(v);
        }
        bail!("Slack kept saying to slow down, for {method}")
    }

    /// Who this session is.
    pub fn whoami(&self) -> anyhow::Result<Who> {
        let v = self.call("auth.test", &[])?;
        let text = |name: &str| v[name].as_str().unwrap_or("").to_string();
        Ok(Who {
            user: text("user"),
            user_id: text("user_id"),
            team: text("team"),
            domain: domain_of(v["url"].as_str().unwrap_or("")),
        })
    }

    /// Every conversation this session can see, its own or not.
    pub fn channels(&self) -> anyhow::Result<Vec<Channel>> {
        self.conversations("conversations.list")
    }

    /// The conversations this session is in.
    pub fn mine(&self) -> anyhow::Result<Vec<Channel>> {
        self.conversations("users.conversations")
    }

    fn conversations(&self, method: &str) -> anyhow::Result<Vec<Channel>> {
        let found = self.paged(
            method,
            &[
                ("types", "public_channel,private_channel,mpim,im"),
                ("exclude_archived", "true"),
            ],
            "channels",
        )?;
        Ok(found.iter().filter_map(Channel::read).collect())
    }

    /// Every page of what `method` answers, the items under `key` of each
    /// together, as Slack's cursor leads from one page to the next.
    fn paged(&self, method: &str, form: &[(&str, &str)], key: &str) -> anyhow::Result<Vec<Value>> {
        let (mut all, mut cursor) = (Vec::new(), String::new());
        loop {
            let mut fields = form.to_vec();
            fields.extend([("limit", "200"), ("cursor", cursor.as_str())]);
            let v = self.call(method, &fields)?;
            if let Some(page) = v[key].as_array() {
                all.extend(page.iter().cloned());
            }
            cursor = v["response_metadata"]["next_cursor"]
                .as_str()
                .unwrap_or("")
                .to_string();
            if cursor.is_empty() {
                return Ok(all);
            }
        }
    }

    /// How many messages of a conversation are unread: as Slack counts them,
    /// where it does; else those since it was last read, up to `UNREAD_MOST`.
    pub fn unread(&self, channel: &Channel) -> anyhow::Result<u32> {
        let v = self.call("conversations.info", &[("channel", &channel.id)])?;
        let info = &v["channel"];
        if let Some(n) = info["unread_count_display"].as_u64() {
            return Ok(n.min(u64::from(u32::MAX)) as u32);
        }
        let Some(last) = info["last_read"].as_str() else {
            return Ok(0);
        };
        let most = UNREAD_MOST.to_string();
        let v = self.call(
            "conversations.history",
            &[("channel", &channel.id), ("oldest", last), ("limit", &most)],
        )?;
        Ok(v["messages"].as_array().map_or(0, Vec::len) as u32)
    }

    /// Send `text` to a conversation as you, in the thread begun by
    /// `thread` if there is one; its ID, as Slack gives it.
    pub fn post(&self, channel: &str, text: &str, thread: Option<&str>) -> anyhow::Result<String> {
        let text = escaped(text);
        let mut form = vec![("channel", channel), ("text", text.as_str())];
        if let Some(ts) = thread {
            form.push(("thread_ts", ts));
        }
        let v = self
            .call("chat.postMessage", &form)
            .map_err(|e| needs(e, "send", "chat:write"))?;
        Ok(v["ts"].as_str().unwrap_or("").to_string())
    }

    /// Send the files at `paths` to a conversation as you, with `text` as
    /// what is said with them, in the thread begun by `thread` if there is
    /// one: as Slack has files sent, each put where it says, then all shared.
    pub fn upload(
        &self,
        channel: &str,
        text: &str,
        thread: Option<&str>,
        paths: &[std::path::PathBuf],
    ) -> anyhow::Result<()> {
        // Each read first: a file not there sends nothing.
        let mut files = Vec::new();
        for path in paths {
            let bytes =
                std::fs::read(path).with_context(|| format!("cannot read {}", path.display()))?;
            if bytes.len() as u64 > FILE_MOST {
                bail!("{} is too big to send from letterbox", path.display());
            }
            let name = path
                .file_name()
                .map(|n| n.to_string_lossy().into_owned())
                .unwrap_or_else(|| "file".to_string());
            files.push((name, bytes));
        }
        let mut shared = Vec::new();
        for (name, bytes) in files {
            let length = bytes.len().to_string();
            let v = self
                .call(
                    "files.getUploadURLExternal",
                    &[("filename", &name), ("length", &length)],
                )
                .map_err(|e| needs(e, "send files", "files:write"))?;
            let (Some(url), Some(id)) = (v["upload_url"].as_str(), v["file_id"].as_str()) else {
                bail!("Slack gave nowhere to send {name}");
            };
            // The address is enough: the token goes nowhere with the bytes.
            if self.base == API && !slacks(url) {
                bail!("Slack gave somewhere not Slack's to send {name}: {url}");
            }
            let answer = self
                .agent
                .post(url)
                .send(&bytes[..])
                .with_context(|| format!("cannot send {name} to Slack"))?;
            if !answer.status().is_success() {
                bail!("Slack gave {} for {name}", answer.status());
            }
            shared.push(serde_json::json!({ "id": id, "title": name }));
        }
        let shared = Value::Array(shared).to_string();
        let comment = escaped(text.trim_end());
        let mut form = vec![("files", shared.as_str()), ("channel_id", channel)];
        if !comment.trim().is_empty() {
            form.push(("initial_comment", comment.as_str()));
        }
        if let Some(ts) = thread {
            form.push(("thread_ts", ts));
        }
        self.call("files.completeUploadExternal", &form)
            .map_err(|e| needs(e, "send files", "files:write"))?;
        Ok(())
    }

    /// A file a message came with, fetched with the token, from Slack alone:
    /// a real session sends its token to no other host, whatever a message
    /// names. A page in its place is Slack asking for a permission.
    pub fn download(&self, url: &str) -> anyhow::Result<Vec<u8>> {
        if self.base == API && !slacks(url) {
            bail!("that file is not Slack's to fetch: {url}");
        }
        let mut answer = self
            .agent
            .get(url)
            .header("Authorization", format!("Bearer {}", self.session.token))
            .call()
            .context("cannot reach Slack for the file")?;
        if !answer.status().is_success() {
            bail!("Slack gave {} for the file", answer.status());
        }
        let page = answer
            .headers()
            .get("content-type")
            .and_then(|v| v.to_str().ok())
            .is_some_and(|t| t.starts_with("text/html"));
        if page {
            bail!(
                "Slack gave a page, not the file: your app may not read files yet. \
                 Add files:read to its User Token Scopes (OAuth & Permissions), reinstall it, \
                 then letterbox slack add"
            );
        }
        answer
            .body_mut()
            .with_config()
            .limit(FILE_MOST)
            .read_to_vec()
            .context("cannot read the file Slack gave")
    }
}
