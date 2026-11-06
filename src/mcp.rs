//! letterbox as an MCP server, for an AI agent to list, search and read
//! mail, and mark it: `letterbox mcp`, run by the agent's program, as
//! Cursor or Claude Code, which speaks to it in JSON-RPC, a message a line,
//! on its stdin and stdout, as paperstack's does. Nothing else is written
//! to stdout.
//!
//! Each account's store is opened as it is first asked of, and kept open.
//! No message is sent or deleted for good here; reading one does not mark
//! it read. A message is moved, to the Trash or under a label, only once
//! the user said yes to it, as letterbox asks them through the agent's
//! program (MCP's elicitation), never as the agent says they did; where
//! the program cannot ask, nothing is moved. What they said yes to is
//! written down, with the map's public key alone, for letterbox to learn.
//! A letter it is asked to write is kept in Drafts, for the user to send;
//! one thrown away from there, as they say yes to, teaches nothing.
//! The map's index is read, never written, as letterbox may be writing it:
//! what is near a message, where it would be filed, and whether it would
//! be thrown away.

mod calls;
mod drafts;
mod map;
mod tools;

pub(crate) use drafts::ATTACHED_AT_MOST;
#[cfg(test)]
pub(crate) use drafts::off_limits;
use map::Mapped;
use tools::{initialize, tools};

use crate::config::{self, Account};
use crate::mail::{self, Folder, Summary};
use crate::store::{Flag, Store};
use crate::{demo, imap, secret};
use anyhow::{Context, bail};
use serde_json::{Value, json};
use std::io::Write;
use std::path::Path;

/// How an agent goes through new mail with letterbox, served as the prompt
/// `mail-triage`: to every agent letterbox is given to, wherever it runs.
const TRIAGE: &str = include_str!("triage.md");
const TRIAGE_PROMPT: &str = "mail-triage";

/// The protocol's version answered when the client's is not one known.
const PROTOCOL: &str = "2025-06-18";
const KNOWN: &[&str] = &["2024-11-05", "2025-03-26", "2025-06-18", "2025-11-25"];
/// How many characters of a message `read_message` gives at once, by
/// default and at most.
const CHARS: usize = 20_000;
const CHARS_LIMIT: usize = 200_000;
/// The most messages listed at once.
const LISTED_LIMIT: usize = 200;
/// The most messages moved at once, each shown to the user to say yes to.
const MOVED_LIMIT: usize = 50;
/// How far back `triage` and `recent_messages` look, by default and at
/// most, in hours.
const HOURS: u64 = 24;
const HOURS_LIMIT: u64 = 24 * 31;
/// How many days to leave between asking the user what letterbox is least
/// sure of.
const ASK_EVERY: i64 = 3;
/// How many pages of 50 `recent` looks through, at most.
const RECENT_PAGES: usize = 20;
/// How long the map, as read, is kept before it is read again, in seconds:
/// letterbox, open, takes in new mail as it comes.
const MAP_FRESH: u64 = 300;

/// An account, its store opened once it is asked of.
struct Opened {
    name: String,
    account: Option<Account>,
    store: Option<Box<dyn Store>>,
}

pub struct Server {
    /// The accounts' file, with how letterbox is opened in a terminal.
    config: std::path::PathBuf,
    accounts: Vec<Opened>,
    demo: bool,
    /// The map's index and what it makes, read as it is first asked of; or
    /// why it cannot be; and when it was read.
    map: Option<(Result<Mapped, String>, std::time::Instant)>,
    /// Where messages come from and replies go, while it serves.
    io: Option<Io>,
    /// Whether the agent's program can ask the user something for it.
    asks: bool,
    /// How many times it has asked, to tell each answer by.
    asked: u64,
    /// What the user said yes to, learnt in memory alone: the demo's.
    pub learnt: Vec<crate::filter::Taught>,
    /// What was reported, kept in memory alone: the demo's.
    pub reports: Vec<crate::report::Report>,
    /// Whether letterbox started the agent, as `LETTERBOX_AGENT` says: no
    /// one is there to say yes, so nothing is moved, only reported.
    pub background: bool,
}

/// The lines it reads and where it writes.
pub struct Io {
    pub lines: Box<dyn Iterator<Item = std::io::Result<String>>>,
    pub out: Box<dyn Write>,
}

impl Server {
    /// The server over the accounts in the file at `config`, or the demo's.
    pub fn new(config: &Path, demo: bool) -> anyhow::Result<Self> {
        let accounts = if demo {
            vec![Opened {
                name: "Demo".to_string(),
                account: None,
                store: Some(Box::new(demo::Demo::new())),
            }]
        } else {
            config::load(config)?
                .into_iter()
                .map(|a| Opened {
                    name: a.name.clone(),
                    account: Some(a),
                    store: None,
                })
                .collect()
        };
        Ok(Server {
            config: config.to_path_buf(),
            accounts,
            demo,
            map: None,
            io: None,
            asks: false,
            asked: 0,
            learnt: Vec::new(),
            reports: Vec::new(),
            background: std::env::var_os("LETTERBOX_AGENT").is_some_and(|v| !v.is_empty()),
        })
    }

    /// Answer messages on stdin until it closes.
    pub fn run(&mut self) -> anyhow::Result<()> {
        self.serve(Io {
            lines: Box::new(std::io::stdin().lines()),
            out: Box::new(std::io::stdout()),
        })
    }

    /// Answer the messages of `io` until they end.
    pub fn serve(&mut self, io: Io) -> anyhow::Result<()> {
        self.io = Some(io);
        while let Some(line) = self.next_line()? {
            if line.trim().is_empty() {
                continue;
            }
            let reply = match serde_json::from_str::<Value>(&line) {
                Ok(Value::Array(batch)) => {
                    let replies: Vec<Value> = batch.iter().filter_map(|m| self.answer(m)).collect();
                    (!replies.is_empty()).then_some(Value::Array(replies))
                }
                Ok(message) => self.answer(&message),
                Err(e) => Some(error(Value::Null, -32700, &format!("not JSON: {e}"))),
            };
            if let Some(reply) = reply {
                self.send(&reply)?;
            }
        }
        Ok(())
    }

    fn next_line(&mut self) -> anyhow::Result<Option<String>> {
        match &mut self.io {
            Some(io) => Ok(io.lines.next().transpose()?),
            None => Ok(None),
        }
    }

    fn send(&mut self, message: &Value) -> anyhow::Result<()> {
        let io = self.io.as_mut().context("not serving")?;
        writeln!(io.out, "{message}")?;
        io.out.flush()?;
        Ok(())
    }

    /// Ask the user, through the agent's program, whether to do what
    /// `message` says; whether they said yes. Asked of them, never of the
    /// agent: where the program cannot ask, it is an error, and nothing is
    /// done.
    fn confirm(&mut self, message: &str, yes: &str) -> anyhow::Result<Said> {
        let schema = json!({ "confirm": { "type": "boolean", "title": yes, "default": true } });
        let result = self.elicit(message, schema, "confirm")?;
        let confirm = result["content"]["confirm"].as_bool();
        Ok(match (result["action"].as_str(), confirm) {
            (Some("accept"), Some(true)) => Said::Yes,
            (Some("accept"), Some(false)) | (Some("decline"), _) => Said::No,
            _ => Said::Unanswered,
        })
    }

    /// Ask the user `message` through the agent's program, for the field
    /// `required` of those `properties`; what their program answers.
    fn elicit(
        &mut self,
        message: &str,
        properties: Value,
        required: &str,
    ) -> anyhow::Result<Value> {
        if self.io.is_none() {
            bail!("{CANNOT_ASK}");
        }
        self.asked += 1;
        let id = format!("letterbox-{}", self.asked);
        self.send(&json!({
            "jsonrpc": "2.0",
            "id": id,
            "method": "elicitation/create",
            "params": {
                "message": message,
                "requestedSchema": {
                    "type": "object",
                    "properties": properties,
                    "required": [required]
                }
            }
        }))?;
        while let Some(line) = self.next_line()? {
            let Ok(got) = serde_json::from_str::<Value>(&line) else {
                continue;
            };
            if got.get("id").and_then(Value::as_str) == Some(id.as_str())
                && got.get("method").is_none()
            {
                return Ok(got["result"].clone());
            }
            // Asked of it while it waits: only a ping is answered.
            if let (Some(asked), Some(method)) = (got.get("id"), got.get("method")) {
                let reply = if method == "ping" {
                    json!({ "jsonrpc": "2.0", "id": asked, "result": {} })
                } else {
                    error(
                        asked.clone(),
                        -32000,
                        "waiting for the user to say yes or no",
                    )
                };
                self.send(&reply)?;
            }
        }
        bail!("the agent's program closed before the user said yes or no")
    }

    /// The reply to `message`; none to a notification.
    pub fn answer(&mut self, message: &Value) -> Option<Value> {
        let id = message.get("id").cloned()?;
        let method = message.get("method").and_then(Value::as_str).unwrap_or("");
        let params = message.get("params").cloned().unwrap_or(Value::Null);
        if method == "initialize" {
            self.asks = params.pointer("/capabilities/elicitation").is_some();
        }
        let result = match method {
            "initialize" => Ok(initialize(&params)),
            "ping" => Ok(json!({})),
            "tools/list" => Ok(json!({ "tools": tools(self.asks, self.background) })),
            "tools/call" => Ok(self.call(&params)),
            "resources/list" => Ok(json!({ "resources": [] })),
            "prompts/list" => Ok(json!({ "prompts": [{
                "name": TRIAGE_PROMPT,
                "title": "Triage new mail",
                "description": "Look through new mail with letterbox: sum it up, propose what to throw away or file, and what to ask next; nothing moved unless the user says yes",
            }] })),
            "prompts/get" => match params.get("name").and_then(Value::as_str) {
                Some(TRIAGE_PROMPT) => Ok(json!({
                    "description": "Triage new mail with letterbox",
                    "messages": [{ "role": "user", "content": { "type": "text", "text": TRIAGE } }],
                })),
                other => Err((-32602, format!("no prompt {}", other.unwrap_or("named")))),
            },
            _ => Err((-32601, format!("no method {method}"))),
        };
        Some(match result {
            Ok(result) => json!({ "jsonrpc": "2.0", "id": id, "result": result }),
            Err((code, why)) => error(id, code, &why),
        })
    }

    /// A tool's result: its text, and what it found as data; a tool that
    /// fails says why, as its result, for the agent to read.
    fn call(&mut self, params: &Value) -> Value {
        let name = params.get("name").and_then(Value::as_str).unwrap_or("");
        let mut args = params
            .get("arguments")
            .cloned()
            .unwrap_or_else(|| json!({}));
        // Null is what some clients send for what they leave out.
        if let Some(given) = args.as_object_mut() {
            given.retain(|_, v| !v.is_null());
        }
        self.forget_stale_map();
        let done = match refused(name, self.asks, self.background) {
            None => self.tool(name, &args),
            Some(why) => Err(anyhow::anyhow!("{why}")),
        };
        match done {
            Ok(data) => {
                let text = match &data {
                    Value::String(s) => s.clone(),
                    other => serde_json::to_string_pretty(other).unwrap_or_default(),
                };
                let mut result =
                    json!({ "content": [{ "type": "text", "text": text }], "isError": false });
                if data.is_object() {
                    result["structuredContent"] = data;
                }
                result
            }
            Err(e) => json!({
                "content": [{ "type": "text", "text": said(&e) }],
                "isError": true
            }),
        }
    }

    /// The first account's store, for a test to change the mail as another
    /// would.
    #[cfg(test)]
    pub fn first_store(&mut self) -> &mut dyn Store {
        self.store(None).expect("an account").as_mut()
    }

    /// The first account's mail kept by `store`, as a test would have it.
    #[cfg(test)]
    pub fn use_store(&mut self, store: Box<dyn Store>) {
        self.accounts[0].store = Some(store);
    }

    /// The store of the account named, or of the first; opened if it is
    /// not yet.
    fn store(&mut self, name: Option<&str>) -> anyhow::Result<&mut Box<dyn Store>> {
        if self.accounts.is_empty() {
            bail!("no accounts are set up: `letterbox accounts add` adds one");
        }
        let at = match name {
            None => 0,
            Some(n) => self
                .accounts
                .iter()
                .position(|a| a.name.eq_ignore_ascii_case(n))
                .with_context(|| {
                    let names: Vec<&str> = self.accounts.iter().map(|a| a.name.as_str()).collect();
                    format!("no account named {n}: there are {}", names.join(", "))
                })?,
        };
        let opened = &mut self.accounts[at];
        if opened.store.is_none() {
            let account = opened.account.clone().expect("an account set up");
            if account.keep == secret::Keeper::Vault
                && account.password_command.is_none()
                && !crate::vault::is_open()
                // A vault with a key file opens without asking.
                && !crate::vault::unlock_by_key().is_some_and(|r| r.is_ok())
            {
                bail!(
                    "{}'s password is in letterbox's vault, whose passphrase cannot be asked for \
                     here: keep it in the keyring, or the vault with a key file, to read it from \
                     an agent",
                    account.name
                );
            }
            opened.store = Some(Box::new(imap::Imap::new(account)));
        }
        Ok(opened.store.as_mut().expect("opened above"))
    }

    /// The account named for the store known as `ident`.
    fn name_of(&mut self, ident: &str) -> String {
        let names: Vec<String> = self.accounts.iter().map(|a| a.name.clone()).collect();
        names
            .into_iter()
            .find(|n| self.store(Some(n)).is_ok_and(|s| s.ident() == ident))
            .unwrap_or_else(|| ident.to_string())
    }

    /// The names of the accounts asked of: the one named, else every one.
    fn names(&self, account: Option<&str>) -> Vec<String> {
        self.accounts
            .iter()
            .map(|a| a.name.clone())
            .filter(|n| account.is_none_or(|a| n.eq_ignore_ascii_case(a)))
            .collect()
    }

    /// What the account named, or else the first, is called.
    fn called(&self, name: Option<&str>) -> Option<String> {
        self.accounts
            .iter()
            .find(|a| name.is_none_or(|n| a.name.eq_ignore_ascii_case(n)))
            .map(|a| a.name.clone())
    }

    /// The messages come to the Inbox of the account named since `since`,
    /// newest first, at most `limit`; only those unread if `unread`.
    fn recent(
        &mut self,
        account: &str,
        since: i64,
        unread: bool,
        limit: usize,
    ) -> anyhow::Result<Vec<Summary>> {
        let store = self.store(Some(account))?;
        let mut found = Vec::new();
        let mut offset = 0;
        // Listed as they came, not by their dates: one put back, or sent
        // late, may be among them dated before. Looked through until a
        // whole page is older, as far as `RECENT_PAGES`.
        for _ in 0..RECENT_PAGES {
            let page = store.page("INBOX", offset, 50)?;
            if page.summaries.is_empty() {
                break;
            }
            offset += page.summaries.len();
            let mut any = false;
            for s in page.summaries {
                if s.date.is_some_and(|d| d < since) {
                    continue;
                }
                any = true;
                if unread && s.flags.seen {
                    continue;
                }
                found.push(s);
                if found.len() >= limit {
                    return Ok(found);
                }
            }
            if !any || offset >= page.total as usize {
                break;
            }
        }
        Ok(found)
    }

    /// The new mail of each account asked of, as `recent` finds it, each
    /// message weighed by the map if `weigh`; an account that cannot be
    /// read says why.
    fn recent_all(&mut self, args: &Value, weigh: bool) -> Value {
        let hours = args
            .get("hours")
            .and_then(Value::as_u64)
            .unwrap_or(HOURS)
            .clamp(1, HOURS_LIMIT);
        let unread = args.get("unread_only").and_then(Value::as_bool) == Some(true);
        let limit = args
            .get("limit")
            .and_then(Value::as_u64)
            .map_or(50, |n| n as usize)
            .clamp(1, LISTED_LIMIT);
        let since = jiff::Timestamp::now().as_second() - (hours * 3600) as i64;
        let asked = args.get("account").and_then(Value::as_str);
        let mut accounts = Vec::new();
        for n in self.names(asked) {
            accounts.push(match self.recent(&n, since, unread, limit) {
                Ok(found) => {
                    let messages: Vec<Value> = found
                        .iter()
                        .map(|s| {
                            if weigh {
                                self.assessed(&n, s)
                            } else {
                                summary(s)
                            }
                        })
                        .collect();
                    json!({ "name": n, "folder": "INBOX", "messages": messages })
                }
                Err(e) => json!({ "name": n, "error": said(&e) }),
            });
        }
        json!({
            "since": date(since),
            "started_by_letterbox": self.background,
            "accounts": accounts,
        })
    }

    /// What the map says of a new message: how likely it is to be thrown
    /// away, and where it would be filed; or why it cannot say.
    fn assessed(&mut self, account: &str, s: &Summary) -> Value {
        let mut v = summary(s);
        let weighed = self
            .summary_on_map(Some(account), "INBOX", s)
            .and_then(|i| {
                let m = self.mapped()?;
                let junk = m
                    .junk(&m.snapshot.dots[i].key)
                    .context("not in the index")?;
                Ok((junk, m.snapshot.suggestion(i).map(str::to_string)))
            });
        match weighed {
            Ok(((odds, sure, why), go)) => {
                v["junk_odds"] = json!(odds);
                v["would_throw_away"] = json!(sure);
                v["junk_why"] = json!(why);
                v["would_go_to"] = json!(go);
            }
            Err(e) => v["map"] = json!(said(&e)),
        }
        v
    }

    /// The messages `uids` of `folder` of the account named, as they are
    /// now, to show the user before they are moved.
    fn moving(
        &mut self,
        account: Option<&str>,
        folder: &str,
        uids: &[u32],
    ) -> anyhow::Result<Vec<Summary>> {
        let got = self.store(account)?.fetch_index(folder, uids)?;
        uids.iter()
            .map(|uid| {
                let g = got
                    .iter()
                    .find(|g| g.uid == *uid)
                    .with_context(|| format!("there is no message {uid} in {folder}"))?;
                Ok(mail::summary(g.uid, g.flags, g.size, &g.raw))
            })
            .collect()
    }

    /// The folder or label `to` of the account named, as it is called
    /// there: one there is, never one made up.
    fn existing(&mut self, folders: &[Folder], to: &str) -> anyhow::Result<String> {
        if let Some(f) = folders.iter().find(|f| f.path == to && f.selectable) {
            // Filed is kept: never where mail is thrown away, sent or new.
            if !matches!(f.role, mail::Role::Other | mail::Role::Archive) {
                bail!("{to} is not a folder to file under: trash_messages moves mail to Trash");
            }
            return Ok(to.to_string());
        }
        let labelled = self.mapped().ok().and_then(|m| {
            m.snapshot
                .model
                .label_vectors
                .values()
                .map(|(name, _)| name)
                .find(|name| name.eq_ignore_ascii_case(to))
                .cloned()
        });
        labelled.with_context(|| {
            format!(
                "there is no folder or label {to}: list_accounts lists the folders, and \
                 suggest_label the labels"
            )
        })
    }

    /// The report `args` give, each message proposed read as it is now:
    /// who sent it and what about as letterbox reads it, never as the
    /// agent says; a label to file one under, one there is.
    fn report(&mut self, args: &Value) -> anyhow::Result<crate::report::Report> {
        use crate::report::{ASKED, Action, PROPOSED, Proposal, Report};
        let summary = args
            .get("summary")
            .and_then(Value::as_str)
            .context("give the summary")?;
        let listed = |k: &str| {
            args.get(k)
                .and_then(Value::as_array)
                .cloned()
                .unwrap_or_default()
        };
        let (proposed, asked) = (listed("proposals"), listed("questions"));
        if proposed.len() > PROPOSED || asked.len() > ASKED {
            bail!("at most {PROPOSED} proposals and {ASKED} questions");
        }
        // Each account's folders listed once, each folder's messages
        // fetched together.
        let mut folders: std::collections::HashMap<Option<String>, Vec<Folder>> =
            Default::default();
        let mut wanted: std::collections::BTreeMap<(Option<String>, String), Vec<u32>> =
            Default::default();
        let mut proposals: Vec<(Option<String>, Proposal)> = Vec::new();
        for p in proposed {
            let text = |k: &str| p.get(k).and_then(Value::as_str).map(str::to_string);
            let account = text("account");
            let folder = text("folder").unwrap_or_else(|| "INBOX".to_string());
            let uid = p
                .get("uid")
                .and_then(Value::as_u64)
                .context("give each proposal's uid")? as u32;
            let action = match text("action").as_deref() {
                Some("trash") => Action::Trash,
                Some("file") => Action::File,
                _ => bail!("each proposal's action is trash or file"),
            };
            if !folders.contains_key(&account) {
                let listed = self.store(account.as_deref())?.folders()?;
                folders.insert(account.clone(), listed);
            }
            let to = self.destination(
                account.as_deref(),
                &folders[&account],
                &folder,
                action,
                text("to").as_deref(),
            )?;
            let to = if action == Action::File {
                to
            } else {
                String::new()
            };
            wanted
                .entry((account.clone(), folder.clone()))
                .or_default()
                .push(uid);
            let ident = self.store(account.as_deref())?.ident();
            proposals.push((
                account,
                Proposal {
                    account: ident,
                    folder,
                    uid,
                    action,
                    to,
                    why: mail::clean(&text("why").unwrap_or_default()),
                    ..Default::default()
                },
            ));
        }
        // Who sent each and what about, as letterbox reads it.
        let mut read = std::collections::HashMap::new();
        for ((account, folder), uids) in wanted {
            for s in self.moving(account.as_deref(), &folder, &uids)? {
                read.insert((account.clone(), folder.clone(), s.uid), s);
            }
        }
        let proposals = proposals
            .into_iter()
            .map(|(account, p)| {
                let s = &read[&(account, p.folder.clone(), p.uid)];
                // Said as it is, whatever the agent says of it.
                let why = format!("{}{}", p.why, marks(s)).trim().to_string();
                Proposal {
                    message_id: s.message_id.clone(),
                    from: mail::clean(&s.from),
                    subject: mail::clean(&s.subject),
                    why,
                    ..p
                }
            })
            .collect();
        Ok(Report {
            at: jiff::Timestamp::now().as_second(),
            summary: mail::clean_lines(summary),
            proposals,
            questions: asked
                .iter()
                .filter_map(Value::as_str)
                .map(mail::clean)
                .collect(),
        })
    }

    /// Where mail of `folder`, of the account named, whose `folders` are
    /// these, goes as `action` says: its Trash, or the folder or label
    /// `to`, one there is; never where it is already.
    fn destination(
        &mut self,
        account: Option<&str>,
        folders: &[Folder],
        folder: &str,
        action: crate::report::Action,
        to: Option<&str>,
    ) -> anyhow::Result<String> {
        let to = match action {
            crate::report::Action::Trash => {
                let chosen = self.store(account)?.trash().map(str::to_string);
                mail::trash_folder(folders, chosen.as_deref())
                    .context("this account has no Trash: choose one in letterbox, with d")?
            }
            crate::report::Action::File => self.existing(
                folders,
                to.context("give the folder or label to file it under")?,
            )?,
        };
        if to == folder {
            bail!("it is in {to} already");
        }
        Ok(to)
    }
}

/// The folders of `store`, with how many messages each holds and how many
/// are unread.
fn folders_counted(store: &mut dyn Store) -> anyhow::Result<Vec<Folder>> {
    let mut folders = store.folders()?;
    for f in folders.iter_mut().filter(|f| f.selectable) {
        if let Ok((total, unseen)) = store.counts(&f.path) {
            f.total = Some(total);
            f.unseen = Some(unseen);
        }
    }
    Ok(folders)
}

/// Why the tool `name` is not offered, if it is not: moves only where the
/// user can be asked to say yes to them; letterbox opened only by an agent
/// it did not start.
fn refused(name: &str, asks: bool, background: bool) -> Option<&'static str> {
    match name {
        "draft_reply" | "draft_message" if background => {
            Some("letterbox started this agent: draft a letter only as the user asks for it")
        }
        "ask_about_messages" if background => Some(
            "letterbox started this agent, and no one is there to answer: say in the summary \
             that L in letterbox asks about what it is least sure of",
        ),
        "ask_about_messages" if !asks => Some(
            "the agent's program cannot ask the user: L in letterbox asks about what it is \
             least sure of",
        ),
        "trash_messages" | "file_messages" if background => Some(
            "letterbox started this agent, and no one is there to say yes, so nothing is \
             moved: report_triage hands letterbox what you would move, for the user to choose",
        ),
        "trash_messages" | "file_messages" if !asks => Some(CANNOT_ASK),
        "open_letterbox" if background => Some("letterbox started this agent: it is open"),
        "mark_message" if background => {
            Some("letterbox started this agent: mark mail only as the user asks")
        }
        _ => None,
    }
}

/// Why nothing is moved by an agent whose program cannot ask the user.
const CANNOT_ASK: &str = "the agent's program cannot ask the user to say yes, so nothing is \
     moved: use one that can (Claude Code can), or do it in letterbox";

/// How letterbox, at `lbx`, is opened in a terminal of its own: as `set`
/// says, `{lbx}` in it where it is; else, on macOS, in Terminal.
pub fn open_command(set: Option<&str>, lbx: &Path) -> Option<String> {
    let quoted = config::shell_quote(&lbx.display().to_string());
    match set {
        Some(c) => Some(c.replace("{lbx}", &quoted)),
        None if cfg!(target_os = "macos") => Some(format!("open -a Terminal {quoted}")),
        None => None,
    }
}

/// What went wrong, as it is told: with nothing a terminal would take as a
/// command.
fn said(e: &anyhow::Error) -> String {
    mail::clean_lines(&format!("{e:#}"))
}

/// What the user said to what they were asked.
enum Said {
    Yes,
    No,
    /// Dismissed, or never seen.
    Unanswered,
}

/// The uids asked to be moved: at least one, at most `MOVED_LIMIT`, each
/// once.
fn uids(args: &Value) -> anyhow::Result<Vec<u32>> {
    let mut uids: Vec<u32> = args
        .get("uids")
        .and_then(Value::as_array)
        .context("give the messages' uids, as a list")?
        .iter()
        .map(|v| v.as_u64().map(|n| n as u32).context("a uid is a number"))
        .collect::<anyhow::Result<_>>()?;
    let mut seen = std::collections::HashSet::new();
    uids.retain(|u| seen.insert(*u));
    if uids.is_empty() || uids.len() > MOVED_LIMIT {
        bail!("give from 1 to {MOVED_LIMIT} uids");
    }
    Ok(uids)
}

/// What the user is asked: `what`, and each message, by who sent it and
/// what about, flagged or answered said.
/// What is marked on `s` that a move would go against, after a space, in
/// brackets; nothing if nothing is.
fn marks(s: &Summary) -> String {
    let mut marks = Vec::new();
    if s.flags.flagged {
        marks.push("flagged");
    }
    if s.flags.answered {
        marks.push("answered");
    }
    if marks.is_empty() {
        String::new()
    } else {
        format!(" ({})", marks.join(", "))
    }
}

fn listing(what: &str, moving: &[Summary], of_drafts: bool) -> String {
    let mut text = what.to_string();
    for s in moving {
        let marks = marks(s);
        let apart = if of_drafts {
            drafts::told_apart(s)
        } else {
            String::new()
        };
        text.push_str(&format!(
            "\n· {}: {}{marks}{apart}",
            mail::clean(&s.from),
            mail::clean(&s.subject)
        ));
    }
    text
}

/// When a message was sent, in this machine's time zone, as RFC 3339.
fn date(seconds: i64) -> String {
    jiff::Timestamp::from_second(seconds)
        .map(|t| {
            t.to_zoned(jiff::tz::TimeZone::system())
                .strftime("%Y-%m-%dT%H:%M:%S%:z")
                .to_string()
        })
        .unwrap_or_default()
}

fn summary(s: &Summary) -> Value {
    json!({
        "uid": s.uid,
        "from": s.from,
        "address": s.address,
        "to": s.to,
        "subject": s.subject,
        "date": s.date.map(date),
        "seen": s.flags.seen,
        "flagged": s.flags.flagged,
        "answered": s.flags.answered,
        "preview": s.preview,
    })
}

fn error(id: Value, code: i64, message: &str) -> Value {
    json!({ "jsonrpc": "2.0", "id": id, "error": { "code": code, "message": message } })
}
