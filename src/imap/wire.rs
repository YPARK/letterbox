//! The connection to an IMAP server: TLS, from the start or by STARTTLS, a
//! folder selected, and what it says read into summaries and flags.
use super::*;

pub(super) const CONNECT: Duration = Duration::from_secs(20);

pub(super) const WAIT: Duration = Duration::from_secs(90);

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
pub(super) fn starttls(tcp: &TcpStream) -> anyhow::Result<()> {
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

pub(super) type Session = imap::Session<Stream>;

/// The folder selected, as SELECT said it was then.
#[derive(Clone, Debug, Default)]
pub(crate) struct Selected {
    pub(super) folder: String,
    /// How many messages it held.
    pub(super) exists: u32,
    pub(super) validity: u32,
    pub(super) next: u32,
    /// Whether it keeps keywords of any name: `\*` among its PERMANENTFLAGS.
    pub(super) keywords: bool,
    pub(super) modseq: Option<u64>,
}

/// Select `folder`, unless it is already, or `fresh` asks for its count now;
/// how many messages it holds.
pub(super) fn select(
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
pub(super) fn flags(fetch: &imap::types::Fetch) -> (mail::Flags, Vec<String>) {
    let names: Vec<String> = fetch.flags().iter().map(|f| f.to_string()).collect();
    flags_named(names.iter().map(String::as_str))
}

pub(super) fn summaries(fetches: &imap::types::Fetches) -> Vec<(u32, Summary)> {
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

pub(super) fn special(attributes: &[NameAttribute]) -> Option<Role> {
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
pub(super) fn astring(words: &str) -> String {
    let words = mail::controls(words);
    format!("\"{}\"", words.replace('\\', "\\\\").replace('"', "\\\""))
}
