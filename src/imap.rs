//! An account's mail on its IMAP server, over TLS in pure Rust: rustls,
//! with RustCrypto's algorithms and Mozilla's roots, as paperstack's HTTPS.
//! One connection, opened when first needed and again if it is lost.

use crate::config::{Account, Auth, Security};
use crate::index::labels;
use crate::index::record::Indexed;
use crate::mail::{self, Folder, Role, Summary};
use crate::oauth;
use crate::store::{Change, Filed, Flag, Kind, Page, Removal, Status, Store, unsupported};
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
pub(crate) fn tls(host: &str, tcp: TcpStream) -> anyhow::Result<Stream> {
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

pub(crate) fn dial(host: &str, port: u16) -> anyhow::Result<TcpStream> {
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
    /// The folder selected, as it was then.
    selected: Option<Selected>,
    /// Whether this connection can `MOVE`. Asked once, and asked again
    /// after the connection is lost.
    can_move: Option<bool>,
    /// What this connection can do, once asked; CONDSTORE is asked for on
    /// it then, where it can be.
    caps: Option<Caps>,
    /// How the server keeps labels, once told.
    kind: Option<Kind>,
}

impl Imap {
    pub fn new(account: Account) -> Self {
        Imap {
            account,
            session: None,
            selected: None,
            can_move: None,
            caps: None,
            kind: None,
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
        mut f: impl FnMut(&mut Session, &mut Option<Selected>) -> imap::Result<R>,
    ) -> anyhow::Result<R> {
        for attempt in 0..2 {
            if self.session.is_none() {
                self.session = Some(self.connect()?);
                self.selected = None;
                self.caps = None;
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
                    self.can_move = None;
                }
                Err(e) => {
                    if matches!(e, imap::Error::Io(_) | imap::Error::ConnectionLost) {
                        self.session = None;
                        self.can_move = None;
                    }
                    return Err(e).with_context(|| self.account.host.clone());
                }
            }
        }
        unreachable!("the second attempt returns")
    }
}

/// The folder selected, as SELECT said it was then.
#[derive(Clone, Debug, Default)]
pub(crate) struct Selected {
    folder: String,
    /// How many messages it held.
    exists: u32,
    validity: u32,
    next: u32,
    /// Whether it keeps keywords of any name: `\*` among its PERMANENTFLAGS.
    keywords: bool,
    modseq: Option<u64>,
}

/// Select `folder`, unless it is already, or `fresh` asks for its count now;
/// how many messages it holds.
fn select(
    s: &mut Session,
    selected: &mut Option<Selected>,
    folder: &str,
    fresh: bool,
) -> imap::Result<u32> {
    match selected {
        Some(sel) if sel.folder == folder && !fresh => Ok(sel.exists),
        _ => {
            *selected = None;
            let m = s.select(folder)?;
            let keywords = m
                .permanent_flags
                .iter()
                .any(|f| matches!(f, imap::types::Flag::MayCreate));
            *selected = Some(Selected {
                folder: folder.to_string(),
                exists: m.exists,
                validity: m.uid_validity.unwrap_or(0),
                next: m.uid_next.unwrap_or(0),
                keywords,
                modseq: m.highest_mod_seq,
            });
            Ok(m.exists)
        }
    }
}

/// Whether a keyword is a mail program's own, not one of yours: `$Junk`
/// and the like, and Junk, NonJunk and NotJunk, as Apple's Mail and
/// Thunderbird keep them.
pub(crate) fn programs(name: &str) -> bool {
    name.starts_with('$')
        || ["junk", "nonjunk", "notjunk"].contains(&name.to_ascii_lowercase().as_str())
}

/// The flags of a message, by their names as a server gives them: those
/// letterbox shows, and its keywords, but for those that are a mail
/// program's own.
pub(crate) fn flags_named<'a>(
    names: impl IntoIterator<Item = &'a str>,
) -> (mail::Flags, Vec<String>) {
    let mut f = mail::Flags::default();
    let mut keywords = Vec::new();
    for name in names {
        match name.to_ascii_lowercase().as_str() {
            "\\seen" => f.seen = true,
            "\\flagged" => f.flagged = true,
            "\\answered" => f.answered = true,
            k if k.starts_with('\\') || k.is_empty() || programs(k) => {}
            _ => keywords.push(name.to_string()),
        }
    }
    (f, keywords)
}

/// A fetched message's flags, and its keywords.
fn flags(fetch: &imap::types::Fetch) -> (mail::Flags, Vec<String>) {
    let names: Vec<String> = fetch.flags().iter().map(|f| f.to_string()).collect();
    flags_named(names.iter().map(String::as_str))
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
                mail::summary(uid, flags(f).0, f.size.unwrap_or(0), &raw),
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
                    let mut folder =
                        Folder::new(n.name(), n.delimiter(), special(n.attributes()), selectable);
                    folder.all = n
                        .attributes()
                        .iter()
                        .any(|a| matches!(a, NameAttribute::All));
                    folder
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

    fn relocate(
        &mut self,
        from: &str,
        uid: u32,
        to: &str,
        message_id: &str,
    ) -> anyhow::Result<u32> {
        // The new UID comes from the server's COPYUID when it sends one.
        // Otherwise it is found by Message-ID. Guessing the highest UID in
        // the destination would name some other message, and a later
        // expunge would remove that one.
        let known = self.can_move;
        let mut learned = None;
        let found = self.run(|s, selected| {
            select(s, selected, from, false)?;
            let id = if message_id.is_empty() {
                message_id_of(s, uid)?
            } else {
                Some(message_id.to_string())
            };
            let Some(id) = id.filter(|s| !s.is_empty()) else {
                return Ok(Found::NoId);
            };
            // MOVE is one step. Without it, copy and then delete, which a
            // server that stops halfway can leave in both folders.
            let movable = match known.or(learned) {
                Some(movable) => movable,
                None => match s.capabilities() {
                    Ok(caps) => {
                        let movable = caps.has_str("MOVE");
                        learned = Some(movable);
                        movable
                    }
                    Err(_) => false,
                },
            };
            let copied = if movable {
                transferred(s, "MOVE", uid, to)?
            } else {
                let copied = transferred(s, "COPY", uid, to)?;
                s.uid_store(uid.to_string(), "+FLAGS.SILENT (\\Deleted)")?;
                s.uid_expunge(uid.to_string())?;
                copied
            };
            // The source lost a message, so its cached count no longer holds.
            *selected = None;
            if let Some(uid) = copied {
                return Ok(Found::Uid(uid));
            }
            select(s, selected, to, true)?;
            let found = s.uid_search(format!("HEADER Message-ID {}", astring(&id)))?;
            Ok(match found.into_iter().max() {
                Some(uid) => Found::Uid(uid),
                None => Found::Lost,
            })
        });
        if self.session.is_some() && self.can_move.is_none() {
            self.can_move = learned;
        }
        match found? {
            Found::Uid(uid) => Ok(uid),
            Found::NoId => bail!("it has no Message-ID, so it was left where it is"),
            Found::Lost => {
                bail!("it was moved, but its new place could not be told, so it was not recorded")
            }
        }
    }

    fn expunge(&mut self, folder: &str, uid: u32) -> anyhow::Result<Removal> {
        self.run(|s, selected| {
            select(s, selected, folder, false)?;
            s.uid_store(uid.to_string(), "+FLAGS.SILENT (\\Deleted)")?;
            let gone = s.uid_expunge(uid.to_string())?;
            if gone.is_empty() {
                return Ok(Removal::Missing);
            }
            // The folder stays selected. Its count drops by what this
            // expunge removed, so the next one does not select again.
            if let Some(sel) = selected.as_mut()
                && sel.folder == folder
            {
                let removed = (&gone).into_iter().count() as u32;
                sel.exists = sel.exists.saturating_sub(removed);
            }
            Ok(Removal::Gone)
        })
    }

    fn append(&mut self, folder: &str, raw: &[u8], seen: bool) -> anyhow::Result<()> {
        self.run(|s, selected| {
            let mut cmd = s.append(folder, raw);
            if seen {
                cmd.flag(imap::types::Flag::Seen);
            }
            cmd.finish()?;
            if selected.as_ref().is_some_and(|sel| sel.folder == folder) {
                *selected = None;
            }
            Ok(())
        })
    }

    fn create(&mut self, path: &str) -> anyhow::Result<()> {
        self.run(|s, _| {
            s.create(path)?;
            Ok(())
        })
    }

    fn address(&self) -> &str {
        &self.account.email
    }

    fn trash(&self) -> Option<&str> {
        self.account.trash.as_deref()
    }

    fn send(&mut self, raw: &[u8]) -> anyhow::Result<String> {
        let sent = crate::smtp::submit(&self.account, raw)?;
        let (host, _) = self.account.submission();
        if crate::smtp::files_sent(&host) || gmail(&self.account.host) {
            return Ok("sent".to_string());
        }
        let folder = match self.sent_folder() {
            Ok(folder) => folder,
            Err(e) => return Ok(format!("sent, and not saved in Sent: {e:#}")),
        };
        if let Err(e) = self.append(&folder, &sent, true) {
            return Ok(format!("sent, and not saved in Sent: {e:#}"));
        }
        Ok("sent".to_string())
    }

    fn twin(&self) -> Option<Box<dyn Store>> {
        Some(Box::new(Imap::new(self.account.clone())))
    }

    fn ident(&self) -> String {
        format!("{}@{}", self.account.user(), self.account.host).to_lowercase()
    }

    fn kind(&mut self) -> anyhow::Result<Kind> {
        if let Some(kind) = self.kind {
            return Ok(kind);
        }
        if self.prepare()?.gmail {
            self.kind = Some(Kind::Gmail);
            return Ok(Kind::Gmail);
        }
        let archive = self
            .folders()?
            .into_iter()
            .find(|f| f.role == Role::Archive && f.selectable)
            .map(|f| f.path);
        let keywords = self.run(|s, selected| {
            let mut all = true;
            for folder in std::iter::once("INBOX").chain(archive.as_deref()) {
                select(s, selected, folder, true)?;
                all &= selected.as_ref().is_some_and(|sel| sel.keywords);
            }
            Ok(all)
        })?;
        let kind = if keywords {
            Kind::Keywords
        } else {
            Kind::Folders
        };
        self.kind = Some(kind);
        Ok(kind)
    }

    fn status(&mut self, folder: &str) -> anyhow::Result<Status> {
        // Selected, not asked its STATUS, as RFC 3501 would not have a
        // folder about to be read asked: SELECT says the same, and the
        // fetches that follow find it selected.
        self.prepare()?;
        self.run(|s, selected| {
            let messages = select(s, selected, folder, true)?;
            let sel = selected.clone().unwrap_or_default();
            Ok(Status {
                messages,
                validity: sel.validity,
                next: sel.next,
                modseq: sel.modseq,
            })
        })
    }

    fn fetch_index(&mut self, folder: &str, uids: &[u32]) -> anyhow::Result<Vec<Indexed>> {
        if uids.is_empty() {
            return Ok(Vec::new());
        }
        let gmail = self.prepare()?.gmail;
        let command = format!("UID FETCH {} {}", uid_set(uids), indexed_items(gmail));
        self.run(|s, selected| {
            select(s, selected, folder, false)?;
            let (buf, _) = s.run(&command)?;
            Ok(fetched(&buf).into_iter().filter_map(indexed).collect())
        })
    }

    fn changed(
        &mut self,
        folder: &str,
        since: Option<u64>,
        uids: &[u32],
    ) -> anyhow::Result<Vec<Change>> {
        let caps = self.prepare()?;
        let labels = if caps.gmail { " X-GM-LABELS" } else { "" };
        let command = match since {
            Some(m) if caps.condstore => {
                format!("UID FETCH 1:* (UID FLAGS{labels}) (CHANGEDSINCE {m})")
            }
            _ if uids.is_empty() => return Ok(Vec::new()),
            _ => format!("UID FETCH {} (UID FLAGS{labels})", uid_set(uids)),
        };
        self.run(|s, selected| {
            select(s, selected, folder, false)?;
            let (buf, _) = s.run(&command)?;
            Ok(fetched(&buf)
                .into_iter()
                .filter_map(|f| {
                    let i = indexed(f)?;
                    Some(Change {
                        uid: i.uid,
                        flags: i.flags,
                        labels: i.labels,
                        modseq: i.modseq,
                    })
                })
                .collect())
        })
    }

    fn uids(&mut self, folder: &str) -> anyhow::Result<Vec<u32>> {
        self.run(|s, selected| {
            select(s, selected, folder, true)?;
            let mut uids: Vec<u32> = s.uid_search("ALL")?.into_iter().collect();
            uids.sort_unstable();
            Ok(uids)
        })
    }

    fn label(&mut self, folder: &str, uid: u32, label: &str, on: bool) -> anyhow::Result<()> {
        let sign = if on { '+' } else { '-' };
        let change = match self.kind()? {
            Kind::Gmail => format!("{sign}X-GM-LABELS.SILENT ({})", astring(label)),
            Kind::Keywords => format!("{sign}FLAGS.SILENT ({})", labels::keyword(label)),
            Kind::Folders => return Err(unsupported("keep a label on a message")),
        };
        self.store_flags(folder, uid, &change)
    }

    fn file(
        &mut self,
        folder: &str,
        uid: u32,
        label: &str,
        message_id: &str,
    ) -> anyhow::Result<Filed> {
        let kind = self.kind()?;
        if kind == Kind::Gmail {
            self.label(folder, uid, label, true)?;
            self.store_flags(folder, uid, "-X-GM-LABELS.SILENT (\\Inbox)")?;
            return Ok(Filed {
                moved: None,
                on_server: true,
            });
        }
        let folders = self.folders()?;
        let role = folders
            .iter()
            .find(|f| f.path == folder)
            .map_or(Role::Other, |f| f.role);
        if kind == Kind::Keywords {
            self.label(folder, uid, label, true)?;
            if role != Role::Inbox {
                return Ok(Filed {
                    moved: None,
                    on_server: true,
                });
            }
            let archive = match folders
                .iter()
                .find(|f| f.role == Role::Archive && f.selectable)
            {
                Some(f) => f.path.clone(),
                None => {
                    self.create("Archive")?;
                    "Archive".to_string()
                }
            };
            let new = self.relocate(folder, uid, &archive, message_id)?;
            return Ok(Filed {
                moved: Some((archive, new)),
                on_server: true,
            });
        }
        // Folders alone: the first label is the folder it is moved to, from
        // the Inbox or Archive; any other, letterbox keeps.
        if !matches!(role, Role::Inbox | Role::Archive) {
            return Ok(Filed {
                moved: None,
                on_server: false,
            });
        }
        let to = match labels::folder_for(&folders, label) {
            Some(f) => f.path.clone(),
            None => {
                let name = labels::folder_name(label.trim());
                self.create(&name)?;
                name
            }
        };
        let new = self.relocate(folder, uid, &to, message_id)?;
        Ok(Filed {
            moved: Some((to, new)),
            on_server: true,
        })
    }

    fn archive(&mut self, folder: &str, uid: u32, message_id: &str) -> anyhow::Result<()> {
        if self.kind()? == Kind::Gmail {
            // On Gmail, out of the Inbox is without its label: All Mail
            // has it still.
            return self.store_flags(folder, uid, "-X-GM-LABELS.SILENT (\\Inbox)");
        }
        crate::store::to_archive(self, folder, uid, message_id)
    }

    fn rename_label(&mut self, old: &str, new: &str) -> anyhow::Result<()> {
        let kind = self.kind()?;
        let folders = self.folders()?;
        let mut done = false;
        // The folder the label is, renamed where it is, under its parent.
        if let Some(f) = labels::folder_for(&folders, old) {
            let parent = f
                .delimiter
                .as_deref()
                .filter(|d| !d.is_empty())
                .and_then(|d| Some((f.path.rfind(d)?, d)));
            let to = match parent {
                Some((at, d)) => format!("{}{d}{}", &f.path[..at], labels::folder_name(new)),
                None => labels::folder_name(new),
            };
            let from = f.path.clone();
            self.run(|s, selected| {
                *selected = None;
                s.rename(&from, &to)
            })?;
            done = true;
        }
        if kind == Kind::Keywords {
            let (from, to) = (labels::keyword(old), labels::keyword(new));
            let mapped: Vec<String> = folders
                .iter()
                .filter(|f| f.selectable && !matches!(f.role, Role::Junk | Role::Trash))
                .map(|f| f.path.clone())
                .collect();
            for folder in mapped {
                let uids = self.run(|s, selected| {
                    select(s, selected, &folder, false)?;
                    Ok(s.uid_search(format!("KEYWORD {from}"))?
                        .into_iter()
                        .collect::<Vec<u32>>())
                })?;
                if uids.is_empty() {
                    continue;
                }
                let set = uid_set(&uids);
                self.run(|s, selected| {
                    select(s, selected, &folder, false)?;
                    s.uid_store(&set, format!("+FLAGS.SILENT ({to})"))?;
                    s.uid_store(&set, format!("-FLAGS.SILENT ({from})"))?;
                    Ok(())
                })?;
                done = true;
            }
        }
        if !done {
            bail!("the server has no label {old}");
        }
        Ok(())
    }
}

/// What a connection can do, as far as the map asks.
#[derive(Clone, Copy, Debug, Default)]
struct Caps {
    /// Gmail's labels and IDs: `X-GM-EXT-1`.
    gmail: bool,
    /// Mod-sequences: CONDSTORE, asked for on the connection.
    condstore: bool,
}

/// What the index fetches of each message: what is listed, and its
/// `List-Id`; on Gmail, its IDs and labels too.
pub(crate) fn indexed_items(gmail: bool) -> String {
    let gmail = if gmail {
        " X-GM-MSGID X-GM-THRID X-GM-LABELS"
    } else {
        ""
    };
    format!(
        "(UID FLAGS RFC822.SIZE BODY.PEEK[HEADER.FIELDS (FROM TO CC SUBJECT DATE MESSAGE-ID \
         IN-REPLY-TO REFERENCES LIST-ID LIST-UNSUBSCRIBE PRECEDENCE CONTENT-TYPE \
         CONTENT-TRANSFER-ENCODING \
         MIME-VERSION)] \
         BODY.PEEK[TEXT]<0.2048>{gmail})"
    )
}

/// `uids` as an IMAP set, runs of them as ranges: `1:4,7,9:12`.
pub(crate) fn uid_set(uids: &[u32]) -> String {
    let mut sorted = uids.to_vec();
    sorted.sort_unstable();
    sorted.dedup();
    let mut parts: Vec<String> = Vec::new();
    let mut i = 0;
    while i < sorted.len() {
        let start = sorted[i];
        let mut end = start;
        while i + 1 < sorted.len() && sorted[i + 1] == end + 1 {
            i += 1;
            end = sorted[i];
        }
        parts.push(if start == end {
            start.to_string()
        } else {
            format!("{start}:{end}")
        });
        i += 1;
    }
    parts.join(",")
}

/// Each `FETCH` in a server's answer, with what it holds: read here, not
/// by the imap crate, which keeps Gmail's IDs to itself.
fn fetched(buf: &[u8]) -> Vec<Vec<imap_proto::AttributeValue<'_>>> {
    let mut out = Vec::new();
    let mut rest = buf;
    while !rest.is_empty() {
        match imap_proto::parser::parse_response(rest) {
            Ok((left, imap_proto::Response::Fetch(_, attributes))) => {
                out.push(attributes);
                rest = left;
            }
            Ok((left, _)) => rest = left,
            Err(_) => break,
        }
    }
    out
}

/// A message as the index takes it, from what was fetched of it.
fn indexed(attributes: Vec<imap_proto::AttributeValue<'_>>) -> Option<Indexed> {
    use imap_proto::{AttributeValue as A, MessageSection, SectionPath};
    let mut i = Indexed::default();
    let (mut uid, mut header, mut text) = (None, Vec::new(), Vec::new());
    for a in attributes {
        match a {
            A::Uid(u) => uid = Some(u),
            A::Flags(names) => {
                let (flags, keywords) = flags_named(names.iter().map(|n| n.as_ref()));
                i.flags = flags;
                i.labels.extend(keywords);
            }
            A::Rfc822Size(n) => i.size = n,
            A::GmailLabels(labels) => i.labels.extend(labels.iter().map(|l| l.to_string())),
            A::GmailMsgId(id) => i.gmail = Some(id),
            A::GmailThrId(id) => i.thread = Some(id),
            A::ModSeq(m) => i.modseq = Some(m),
            A::BodySection {
                section: Some(SectionPath::Full(MessageSection::Header)),
                data: Some(d),
                ..
            } => header = d.to_vec(),
            A::BodySection {
                section: Some(SectionPath::Full(MessageSection::Text)),
                data: Some(d),
                ..
            } => text = d.to_vec(),
            _ => {}
        }
    }
    i.uid = uid?;
    header.extend(text);
    i.raw = header;
    Some(i)
}

impl Imap {
    /// Change the flags or labels of the message `uid` of `folder`, as
    /// `change` says: `+FLAGS.SILENT (Grant)`, say.
    fn store_flags(&mut self, folder: &str, uid: u32, change: &str) -> anyhow::Result<()> {
        self.run(|s, selected| {
            select(s, selected, folder, false)?;
            s.uid_store(uid.to_string(), change)?;
            Ok(())
        })
    }

    /// What the connection can do, asked once a connection; CONDSTORE is
    /// asked for where the server has it, so that SELECT says a folder's
    /// mod-sequence.
    fn prepare(&mut self) -> anyhow::Result<Caps> {
        if let Some(caps) = self.caps
            && self.session.is_some()
        {
            return Ok(caps);
        }
        let caps = self.run(|s, selected| {
            let c = s.capabilities()?;
            let caps = Caps {
                gmail: c.has_str("X-GM-EXT-1"),
                condstore: c.has_str("CONDSTORE"),
            };
            if caps.condstore && c.has_str("ENABLE") {
                // Without it, CHANGEDSINCE still asks for it, as RFC 7162
                // has it; SELECT then says no mod-sequence.
                let _ = s.run_command_and_check_ok("ENABLE CONDSTORE");
                *selected = None;
            }
            Ok(caps)
        })?;
        self.caps = Some(caps);
        Ok(caps)
    }

    /// The Sent folder, made if the server has none.
    fn sent_folder(&mut self) -> anyhow::Result<String> {
        let folders = self.folders()?;
        if let Some(folder) = folders
            .into_iter()
            .find(|f| f.role == Role::Sent && f.selectable)
        {
            return Ok(folder.path);
        }
        self.create("Sent")?;
        Ok("Sent".to_string())
    }
}

fn gmail(host: &str) -> bool {
    matches!(
        host.to_ascii_lowercase().as_str(),
        "imap.gmail.com" | "imap.googlemail.com"
    )
}

/// Where a moved message was found again, or why it was not.
enum Found {
    Uid(u32),
    /// It has no `Message-ID`, and was left unmoved.
    NoId,
    /// It was moved, and the search did not find it.
    Lost,
}

/// The one UID `COPYUID` names as the copy, when the response has one.
fn copied_uid(response: &[u8]) -> Option<u32> {
    let text = String::from_utf8_lossy(response);
    let rest = text.split("[COPYUID ").nth(1)?;
    let inside = rest.split(']').next()?;
    let mut parts = inside.split_whitespace();
    parts.next()?;
    parts.next()?;
    let dest = parts.next()?;
    if parts.next().is_some() {
        return None;
    }
    if let Ok(uid) = dest.parse() {
        return Some(uid);
    }
    let (start, end) = dest.split_once(':')?;
    let start: u32 = start.parse().ok()?;
    let end: u32 = end.parse().ok()?;
    (start == end).then_some(start)
}

/// `UID MOVE` or `UID COPY`, and the UID the copy was given when the
/// server says it. `uid_mv` checks the response and then drops it.
fn transferred(s: &mut Session, command: &str, uid: u32, to: &str) -> imap::Result<Option<u32>> {
    let (buf, _) = s.run(format!("UID {command} {uid} {}", astring(to)))?;
    Ok(copied_uid(&buf))
}

/// The message's `Message-ID`, from its header only. The search for a
/// moved message uses it when the server did not send `COPYUID`.
fn message_id_of(s: &mut Session, uid: u32) -> imap::Result<Option<String>> {
    let fetches = s.uid_fetch(uid.to_string(), "BODY.PEEK[HEADER]")?;
    let raw = fetches.iter().find_map(|f| f.header()).unwrap_or_default();
    Ok(mail::field(raw, "Message-ID"))
}

impl Drop for Imap {
    fn drop(&mut self) {
        if let Some(mut s) = self.session.take() {
            let _ = s.logout();
        }
    }
}
