//! paperstack, the library of papers, asked over MCP: whether a paper is
//! in it, or like those starred in it, and a paper added to it, starred.
//! letterbox runs `paperstack mcp` (`$LETTERBOX_PAPERSTACK` names another
//! program) and speaks to it one JSON line a message, as it is spoken to.

use crate::papers::{Paper, fold};
use anyhow::{Context, anyhow};
use serde_json::{Value, json};
use std::io::{BufRead, Write};

/// A paper as the library has it.
#[derive(Clone, Debug, PartialEq)]
pub struct Found {
    pub key: String,
    pub title: String,
    pub starred: bool,
}

/// What a library does for letterbox.
pub trait Library: Send {
    /// The papers `query` finds, as paperstack's search reads it.
    fn search(&mut self, query: &str, limit: usize) -> anyhow::Result<Vec<Found>>;
    /// Add the paper `what` names: a DOI, an arXiv id, an address or a title.
    fn add(&mut self, what: &str) -> anyhow::Result<Found>;
    /// Star the paper `key`.
    fn star(&mut self, key: &str) -> anyhow::Result<()>;
}

/// Where a paper a message tells of stands with the library.
#[derive(Clone, Debug, PartialEq)]
pub enum Standing {
    /// In it, by its key, starred or not.
    Have {
        key: String,
        starred: bool,
    },
    /// Not in it, but like a paper starred there, by its title.
    Like(String),
    New,
}

/// Where `paper` stands: in the library by its title; else like a paper
/// starred there, that two of its rarest words find; else new.
pub fn standing(lib: &mut dyn Library, paper: &Paper) -> anyhow::Result<Standing> {
    let title = paper.title.replace('"', " ");
    let want = fold(&paper.title);
    if let Some(f) = lib
        .search(&format!("title:\"{title}\""), 5)?
        .into_iter()
        .find(|f| fold(&f.title) == want)
    {
        return Ok(Standing::Have {
            key: f.key,
            starred: f.starred,
        });
    }
    let mut words: Vec<String> = crate::model::words(&paper.title)
        .into_iter()
        .filter(|w| w.chars().count() >= 6)
        .collect();
    words.sort_by_key(|w| std::cmp::Reverse(w.chars().count()));
    words.dedup();
    let mut hits: Vec<(Found, usize)> = Vec::new();
    for w in words.iter().take(4) {
        for f in lib.search(&format!("is:starred {w}"), 10)? {
            match hits.iter_mut().find(|(h, _)| h.key == f.key) {
                Some((_, n)) => *n += 1,
                None => hits.push((f, 1)),
            }
        }
    }
    Ok(hits
        .into_iter()
        .filter(|(_, n)| *n >= 2)
        .max_by_key(|(_, n)| *n)
        .map_or(Standing::New, |(f, _)| Standing::Like(f.title)))
}

/// paperstack's MCP server, on what it reads from and writes to.
pub struct Mcp<R, W> {
    reader: R,
    writer: W,
    next: u64,
    /// The program, when letterbox runs it: stopped as this is let go.
    child: Option<Running>,
}

/// The program letterbox ran, stopped once it is no longer asked.
struct Running(std::process::Child);

impl Drop for Running {
    fn drop(&mut self) {
        let _ = self.0.kill();
        let _ = self.0.wait();
    }
}

impl<R: BufRead + Send, W: Write + Send> Mcp<R, W> {
    /// The server on `reader` and `writer`, begun as MCP begins.
    pub fn new(reader: R, writer: W) -> anyhow::Result<Self> {
        let mut m = Mcp {
            reader,
            writer,
            next: 0,
            child: None,
        };
        m.request(
            "initialize",
            json!({
                "protocolVersion": "2025-06-18",
                "capabilities": {},
                "clientInfo": { "name": "letterbox", "version": env!("CARGO_PKG_VERSION") }
            }),
        )?;
        m.send(&json!({ "jsonrpc": "2.0", "method": "notifications/initialized" }))?;
        Ok(m)
    }

    /// What was written, for a test.
    #[cfg(test)]
    pub fn into_writer(self) -> W {
        self.writer
    }

    fn send(&mut self, message: &Value) -> anyhow::Result<()> {
        writeln!(self.writer, "{message}")?;
        self.writer.flush()?;
        Ok(())
    }

    /// Ask `method`, and the answer to it: the result, or why not.
    fn request(&mut self, method: &str, params: Value) -> anyhow::Result<Value> {
        self.next += 1;
        let id = self.next;
        self.send(&json!({ "jsonrpc": "2.0", "id": id, "method": method, "params": params }))?;
        loop {
            let mut line = String::new();
            if self.reader.read_line(&mut line)? == 0 {
                anyhow::bail!("paperstack stopped answering");
            }
            if line.trim().is_empty() {
                continue;
            }
            let reply: Value = serde_json::from_str(&line).context("paperstack said no JSON")?;
            // Its notices, and answers to anything else, are let pass.
            if reply.get("id").and_then(Value::as_u64) != Some(id) {
                continue;
            }
            if let Some(e) = reply.get("error") {
                anyhow::bail!(
                    "paperstack: {}",
                    e.get("message")
                        .and_then(Value::as_str)
                        .unwrap_or("an error")
                );
            }
            return Ok(reply.get("result").cloned().unwrap_or(Value::Null));
        }
    }

    /// Call the tool `name` with `arguments`: what it gives back.
    fn call(&mut self, name: &str, arguments: Value) -> anyhow::Result<Value> {
        let r = self.request(
            "tools/call",
            json!({ "name": name, "arguments": arguments }),
        )?;
        let said = r["content"][0]["text"].as_str().unwrap_or_default();
        if r["isError"].as_bool() == Some(true) {
            return Err(anyhow!("paperstack: {said}"));
        }
        Ok(r.get("structuredContent")
            .cloned()
            .unwrap_or_else(|| Value::String(said.to_string())))
    }
}

/// A paper as paperstack's tools give it.
fn found(v: &Value) -> Option<Found> {
    Some(Found {
        key: v.get("key")?.as_str()?.to_string(),
        title: v
            .get("title")
            .and_then(Value::as_str)
            .unwrap_or_default()
            .to_string(),
        starred: v.get("starred").and_then(Value::as_bool).unwrap_or(false),
    })
}

impl<R: BufRead + Send, W: Write + Send> Library for Mcp<R, W> {
    fn search(&mut self, query: &str, limit: usize) -> anyhow::Result<Vec<Found>> {
        let r = self.call("search_papers", json!({ "query": query, "limit": limit }))?;
        Ok(r["papers"]
            .as_array()
            .map(|a| a.iter().filter_map(found).collect())
            .unwrap_or_default())
    }

    fn add(&mut self, what: &str) -> anyhow::Result<Found> {
        let r = self.call("add_paper", json!({ "what": what }))?;
        found(&r["paper"]).ok_or_else(|| {
            anyhow!(
                "paperstack added nothing: {}",
                r["outcome"].as_str().unwrap_or("it did not say why")
            )
        })
    }

    fn star(&mut self, key: &str) -> anyhow::Result<()> {
        self.call("update_paper", json!({ "key": key, "starred": true }))?;
        Ok(())
    }
}

/// The library: `paperstack mcp` run, or the program `$LETTERBOX_PAPERSTACK`
/// names.
pub fn connect() -> anyhow::Result<Box<dyn Library>> {
    let program = std::env::var("LETTERBOX_PAPERSTACK")
        .ok()
        .filter(|p| !p.is_empty())
        .unwrap_or_else(|| "paperstack".to_string());
    let mut child = std::process::Command::new(&program)
        .arg("mcp")
        .stdin(std::process::Stdio::piped())
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::null())
        .spawn()
        .with_context(|| {
            format!("cannot run {program}: is paperstack installed? (cargo install --git https://github.com/YPARK/paperstack)")
        })?;
    let stdout = child.stdout.take().context("paperstack's output")?;
    let stdin = child.stdin.take().context("paperstack's input")?;
    let mut m = Mcp::new(std::io::BufReader::new(stdout), stdin)?;
    m.child = Some(Running(child));
    Ok(Box::new(m))
}
