//! An account's mail on its IMAP server, over TLS in pure Rust: rustls,
//! with RustCrypto's algorithms and Mozilla's roots, as paperstack's HTTPS.
//! One connection, opened when first needed and again if it is lost.

use crate::config::{Account, Auth, Security};
use crate::mail::{self, Folder, Role, Summary};
use crate::oauth;
use crate::store::{Flag, Page, Store};
use anyhow::{Context, bail};
use imap_proto::NameAttribute;
use std::io::{BufRead, BufReader, Read, Write};
use std::net::{TcpStream, ToSocketAddrs};
use std::sync::Arc;
use std::time::Duration;

const CONNECT: Duration = Duration::from_secs(20);
const WAIT: Duration = Duration::from_secs(90);

/// What is fetched of each message listed: its flags and size, the
/// headers shown, and the start of its text to preview it by, with the
/// headers needed to decode that.
const LISTED: &str = "(UID FLAGS RFC822.SIZE BODY.PEEK[HEADER.FIELDS (FROM TO CC SUBJECT DATE \
                      MESSAGE-ID CONTENT-TYPE CONTENT-TRANSFER-ENCODING MIME-VERSION)] \
                      BODY.PEEK[TEXT]<0.2048>)";

pub enum Stream {
    Plain(TcpStream),
    Tls(Box<rustls::StreamOwned<rustls::ClientConnection, TcpStream>>),
}

impl Read for Stream {
    fn read(&mut self, buf: &mut [u8]) -> std::io::Result<usize> {
        match self {
            Stream::Plain(s) => s.read(buf),
            Stream::Tls(s) => s.read(buf),
        }
    }
}

impl Write for Stream {
    fn write(&mut self, buf: &[u8]) -> std::io::Result<usize> {
        match self {
            Stream::Plain(s) => s.write(buf),
            Stream::Tls(s) => s.write(buf),
        }
    }

    fn flush(&mut self) -> std::io::Result<()> {
        match self {
            Stream::Plain(s) => s.flush(),
            Stream::Tls(s) => s.flush(),
        }
    }
}

/// `tcp`, over TLS to `host`.
fn tls(host: &str, tcp: TcpStream) -> anyhow::Result<Stream> {
    // Once a process; again, it is already the provider.
    let _ = rustls_rustcrypto::provider().install_default();
    let roots = rustls::RootCertStore {
        roots: webpki_roots::TLS_SERVER_ROOTS.to_vec(),
    };
    let config = rustls::ClientConfig::builder()
        .with_root_certificates(roots)
        .with_no_client_auth();
    let name = rustls::pki_types::ServerName::try_from(host.to_string())
        .with_context(|| format!("{host} is not a host's name"))?;
    let conn = rustls::ClientConnection::new(Arc::new(config), name)?;
    Ok(Stream::Tls(Box::new(rustls::StreamOwned::new(conn, tcp))))
}

fn dial(host: &str, port: u16) -> anyhow::Result<TcpStream> {
    let mut last = None;
    for addr in (host, port)
        .to_socket_addrs()
        .with_context(|| format!("cannot find {host}"))?
    {
        match TcpStream::connect_timeout(&addr, CONNECT) {
            Ok(tcp) => {
                tcp.set_read_timeout(Some(WAIT))?;
                tcp.set_write_timeout(Some(WAIT))?;
                return Ok(tcp);
            }
            Err(e) => last = Some(e),
        }
    }
    match last {
        Some(e) => Err(e).with_context(|| format!("cannot reach {host}:{port}")),
        None => bail!("cannot find {host}"),
    }
}

/// Ask a server spoken to in plain for TLS, as RFC 3501 has it: its
/// greeting read, `STARTTLS` sent, and its answer awaited.
fn starttls(tcp: &TcpStream) -> anyhow::Result<()> {
    let mut reader = BufReader::new(tcp.try_clone()?);
    let mut line = String::new();
    reader.read_line(&mut line)?;
    if !line.starts_with("* OK") {
        bail!("the server greeted letterbox with {}", line.trim());
    }
    (&*tcp).write_all(b"s1 STARTTLS\r\n")?;
    loop {
        line.clear();
        if reader.read_line(&mut line)? == 0 {
            bail!("the server hung up when asked for TLS");
        }
        if let Some(answer) = line.strip_prefix("s1 ") {
            if answer.starts_with("OK") {
                return Ok(());
            }
            bail!("the server would not start TLS: {}", answer.trim());
        }
    }
}

type Session = imap::Session<Stream>;

pub struct Imap {
    account: Account,
    session: Option<Session>,
    /// The folder selected, and how many messages it held then.
    selected: Option<(String, u32)>,
}

impl Imap {
    pub fn new(account: Account) -> Self {
        Imap {
            account,
            session: None,
            selected: None,
        }
    }

    fn connect(&self) -> anyhow::Result<Session> {
        let a = &self.account;
        let tcp = dial(&a.host, a.port())?;
        let stream = match a.security {
            Security::Tls => tls(&a.host, tcp)?,
            Security::Starttls => {
                starttls(&tcp)?;
                tls(&a.host, tcp)?
            }
            Security::None => Stream::Plain(tcp),
        };
        let mut client = imap::Client::new(stream);
        if a.security != Security::Starttls {
            client
                .read_greeting()
                .with_context(|| format!("{} did not greet letterbox", a.host))?;
        }
        let signed = match a.auth {
            Auth::Password => client.login(a.user(), a.password()?),
            Auth::Microsoft => {
                let token = oauth::access(a)?;
                client.authenticate(
                    "XOAUTH2",
                    &oauth::XOAuth2 {
                        user: a.user().to_string(),
                        token,
                    },
                )
            }
        };
        signed
            .map_err(|(e, _)| e)
            .with_context(|| format!("cannot sign in to {} as {}", a.host, a.user()))
    }

    /// Run `f` in the session, opened first if it is not; once more, on a
    /// new connection, if the old one was lost.
    fn run<R>(
        &mut self,
        mut f: impl FnMut(&mut Session, &mut Option<(String, u32)>) -> imap::Result<R>,
    ) -> anyhow::Result<R> {
        for attempt in 0..2 {
            if self.session.is_none() {
                self.session = Some(self.connect()?);
                self.selected = None;
            }
            let session = self.session.as_mut().expect("opened above");
            let done = f(session, &mut self.selected);
            // Nothing asked for them: what the server says unasked is let go.
            let _ = session.take_all_unsolicited();
            match done {
                Ok(r) => return Ok(r),
                Err(imap::Error::Io(_) | imap::Error::ConnectionLost | imap::Error::Bye(_))
                    if attempt == 0 =>
                {
                    self.session = None;
                }
                Err(e) => {
                    if matches!(e, imap::Error::Io(_) | imap::Error::ConnectionLost) {
                        self.session = None;
                    }
                    return Err(e).with_context(|| self.account.host.clone());
                }
            }
        }
        unreachable!("the second attempt returns")
    }
}

/// Select `folder`, unless it is already, or `fresh` asks for its count now;
/// how many messages it holds.
fn select(
    s: &mut Session,
    selected: &mut Option<(String, u32)>,
    folder: &str,
    fresh: bool,
) -> imap::Result<u32> {
    match selected {
        Some((f, n)) if f == folder && !fresh => Ok(*n),
        _ => {
            *selected = None;
            let n = s.select(folder)?.exists;
            *selected = Some((folder.to_string(), n));
            Ok(n)
        }
    }
}

fn flags(fetch: &imap::types::Fetch) -> mail::Flags {
    let mut f = mail::Flags::default();
    for flag in fetch.flags() {
        match flag {
            imap::types::Flag::Seen => f.seen = true,
            imap::types::Flag::Flagged => f.flagged = true,
            imap::types::Flag::Answered => f.answered = true,
            _ => {}
        }
    }
    f
}

fn summaries(fetches: &imap::types::Fetches) -> Vec<(u32, Summary)> {
    fetches
        .iter()
        .filter_map(|f| {
            let uid = f.uid?;
            let mut raw = f.header().unwrap_or_default().to_vec();
            raw.extend_from_slice(f.text().unwrap_or_default());
            Some((
                f.message,
                mail::summary(uid, flags(f), f.size.unwrap_or(0), &raw),
            ))
        })
        .collect()
}

fn special(attributes: &[NameAttribute]) -> Option<Role> {
    attributes.iter().find_map(|a| match a {
        NameAttribute::Drafts => Some(Role::Drafts),
        NameAttribute::Sent => Some(Role::Sent),
        NameAttribute::Archive => Some(Role::Archive),
        NameAttribute::Junk => Some(Role::Junk),
        NameAttribute::Trash => Some(Role::Trash),
        _ => None,
    })
}

/// `words` as an IMAP quoted string: control characters as spaces, as a
/// quoted string cannot hold a line's end, which would end the command and
/// start another.
fn astring(words: &str) -> String {
    let words = mail::controls(words);
    format!("\"{}\"", words.replace('\\', "\\\\").replace('"', "\\\""))
}

impl Store for Imap {
    fn folders(&mut self) -> anyhow::Result<Vec<Folder>> {
        let mut folders = self.run(|s, _| {
            let names = s.list(Some(""), Some("*"))?;
            Ok(names
                .iter()
                .map(|n| {
                    let selectable = !n
                        .attributes()
                        .iter()
                        .any(|a| matches!(a, NameAttribute::NoSelect));
                    Folder::new(n.name(), n.delimiter(), special(n.attributes()), selectable)
                })
                .collect::<Vec<_>>())
        })?;
        mail::arrange(&mut folders);
        Ok(folders)
    }

    fn counts(&mut self, folder: &str) -> anyhow::Result<(u32, u32)> {
        self.run(|s, _| {
            let m = s.status(folder, "(MESSAGES UNSEEN)")?;
            Ok((m.exists, m.unseen.unwrap_or(0)))
        })
    }

    fn page(&mut self, folder: &str, offset: usize, n: usize) -> anyhow::Result<Page> {
        self.run(|s, selected| {
            let total = select(s, selected, folder, offset == 0)?;
            let hi = (total as usize).saturating_sub(offset);
            if hi == 0 || n == 0 {
                return Ok(Page {
                    summaries: Vec::new(),
                    total,
                });
            }
            let lo = hi.saturating_sub(n - 1).max(1);
            let fetches = s.fetch(format!("{lo}:{hi}"), LISTED)?;
            let mut listed = summaries(&fetches);
            listed.sort_by_key(|l| std::cmp::Reverse(l.0));
            Ok(Page {
                summaries: listed.into_iter().map(|(_, s)| s).collect(),
                total,
            })
        })
    }

    fn raw(&mut self, folder: &str, uid: u32) -> anyhow::Result<Vec<u8>> {
        let raw = self.run(|s, selected| {
            select(s, selected, folder, false)?;
            let fetches = s.uid_fetch(uid.to_string(), "BODY.PEEK[]")?;
            Ok(fetches.iter().find_map(|f| f.body().map(<[u8]>::to_vec)))
        })?;
        raw.ok_or_else(|| anyhow::anyhow!("no message {uid} in {folder}"))
    }

    fn set(&mut self, folder: &str, uid: u32, flag: Flag, on: bool) -> anyhow::Result<()> {
        let sign = if on { '+' } else { '-' };
        self.run(|s, selected| {
            select(s, selected, folder, false)?;
            s.uid_store(
                uid.to_string(),
                format!("{sign}FLAGS.SILENT ({})", flag.imap()),
            )?;
            Ok(())
        })
    }

    fn search(&mut self, folder: &str, words: &str, n: usize) -> anyhow::Result<Vec<Summary>> {
        let query = astring(words);
        self.run(|s, selected| {
            select(s, selected, folder, false)?;
            let uids = match s.uid_search(format!("CHARSET UTF-8 TEXT {query}")) {
                Ok(u) => u,
                // Not every server takes a charset.
                Err(imap::Error::Bad(_) | imap::Error::No(_)) => {
                    s.uid_search(format!("TEXT {query}"))?
                }
                Err(e) => return Err(e),
            };
            let mut uids: Vec<u32> = uids.into_iter().collect();
            uids.sort_unstable_by(|a, b| b.cmp(a));
            uids.truncate(n);
            if uids.is_empty() {
                return Ok(Vec::new());
            }
            let set: Vec<String> = uids.iter().map(u32::to_string).collect();
            let fetches = s.uid_fetch(set.join(","), LISTED)?;
            let mut found: Vec<Summary> = summaries(&fetches).into_iter().map(|(_, s)| s).collect();
            found.sort_by_key(|s| std::cmp::Reverse(s.uid));
            Ok(found)
        })
    }
}

impl Drop for Imap {
    fn drop(&mut self) {
        if let Some(mut s) = self.session.take() {
            let _ = s.logout();
        }
    }
}
