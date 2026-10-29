//! A message's files and links: opened or presented; saved, in `saving`.
use super::*;

/// The message's attachments and links, listed to open, the one picked,
/// and what came of the last opened.
pub struct Files {
    pub items: Vec<Item>,
    pub at: usize,
    pub note: Option<String>,
    /// Its attachments alone, listed to save, not to open.
    pub saving: bool,
}

/// Whether ohp presents a file of this name: PDFs, and markdown and R
/// Markdown, which it typesets.
pub fn ohp_reads(name: &str) -> bool {
    let name = name.to_lowercase();
    [".pdf", ".md", ".markdown", ".rmd", ".html", ".htm"]
        .iter()
        .any(|e| name.ends_with(e))
}

pub(super) fn open(path: &std::path::Path) -> anyhow::Result<()> {
    let program = if cfg!(target_os = "macos") {
        "open"
    } else {
        "xdg-open"
    };
    let status = std::process::Command::new(program)
        .arg(path)
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .status()?;
    if !status.success() {
        anyhow::bail!("{program}: {status}");
    }
    Ok(())
}

/// `html` with each image it shows from its own parts, `images`, by
/// Content-ID, MIME type and bytes, put in it.
pub(crate) fn inline_images(html: &str, images: &[(String, String, Vec<u8>)]) -> String {
    use base64::Engine;
    let mut html = html.to_string();
    for (id, mime, bytes) in images {
        let data = base64::engine::general_purpose::STANDARD.encode(bytes);
        let (needle, url) = (format!("cid:{id}"), format!("data:{mime};base64,{data}"));
        // Its name whole, not the start of another's.
        let mut out = String::with_capacity(html.len());
        let mut rest = html.as_str();
        while let Some(at) = rest.find(&needle) {
            let after = &rest[at + needle.len()..];
            let longer = after
                .chars()
                .next()
                .is_some_and(|c| c.is_alphanumeric() || "._-@$%+".contains(c));
            out.push_str(&rest[..at]);
            out.push_str(if longer { &needle } else { &url });
            rest = after;
        }
        out.push_str(rest);
        html = out;
    }
    html
}

/// The message as a page, written where only you may read it: its HTML,
/// the images it shows from its own parts put in it, or its text; and the
/// file it is in.
pub(super) fn write_page(m: &Message) -> anyhow::Result<PathBuf> {
    let body = match &m.html {
        Some(html) => inline_images(html, &mail::inline_images(&m.raw)),
        None => format!(
            "<meta charset=utf-8><pre style='white-space: pre-wrap'>{}</pre>",
            escape(&m.text)
        ),
    };
    // Mail is from strangers: its scripts, plugins and forms are not run
    // from a file that could reach this machine's other files.
    let html = format!(
        "<meta http-equiv=\"Content-Security-Policy\" content=\"script-src 'none'; \
         object-src 'none'; base-uri 'none'; form-action 'none'\">{body}"
    );
    let dir = cache().join("html");
    let path = dir.join(format!("{}.html", m.uid));
    private_dir(&dir)?;
    vault::replace_private(&path, html.as_bytes())?;
    Ok(path)
}

/// Keep `bytes`, an attachment or a document downloaded, of the message
/// `uid`, in a private folder of the cache, to be opened from there.
pub(super) fn keep_file(uid: u32, name: &str, bytes: &[u8]) -> anyhow::Result<PathBuf> {
    let dir = cache().join("files");
    private_dir(&dir)?;
    let path = dir.join(format!("{uid}-{}", safe_name(name)));
    vault::replace_private(&path, bytes)?;
    Ok(path)
}

/// `$LETTERBOX_DOWNLOADS`, else `~/Downloads`.
pub(super) fn downloads() -> PathBuf {
    std::env::var_os("LETTERBOX_DOWNLOADS")
        .filter(|d| !d.is_empty())
        .map(PathBuf::from)
        .unwrap_or_else(|| {
            PathBuf::from(std::env::var_os("HOME").unwrap_or_default()).join("Downloads")
        })
}

/// Where a picker begins: in `last`, the folder it was last left in, if
/// it is still there; else in the downloads folder, if there is one; else
/// at home.
pub(super) fn start_dir(last: Option<PathBuf>) -> PathBuf {
    last.filter(|d| d.is_dir())
        .or_else(|| Some(downloads()).filter(|d| d.is_dir()))
        .unwrap_or_else(|| PathBuf::from(std::env::var_os("HOME").unwrap_or_default()))
}

/// `$XDG_CACHE_HOME/letterbox`, else `~/.cache/letterbox`: in your home,
/// not the temporary folder others on the machine may write in too.
pub(super) fn cache() -> PathBuf {
    std::env::var_os("XDG_CACHE_HOME")
        .filter(|d| !d.is_empty())
        .map(PathBuf::from)
        .unwrap_or_else(|| {
            PathBuf::from(std::env::var_os("HOME").unwrap_or_default()).join(".cache")
        })
        .join("letterbox")
}

/// `dir`, made if it is not there, only you may look in, where it is made.
pub(super) fn private_dir(dir: &std::path::Path) -> std::io::Result<()> {
    let mut builder = std::fs::DirBuilder::new();
    builder.recursive(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::DirBuilderExt;
        builder.mode(0o700);
    }
    builder.create(dir)
}

/// Write `bytes` to a file not yet at `path`: nothing there is written
/// over, nor a link there followed.
pub(super) fn write_new(path: &std::path::Path, bytes: &[u8]) -> std::io::Result<()> {
    use std::io::Write;
    std::fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(path)?
        .write_all(bytes)
}

/// An attachment's name as a file's: no folders in it, as a sender might
/// try, nothing hidden, and none of the marks that turn text around, as
/// `invoice\u{202e}fdp.exe` shown as `invoiceexe.pdf`.
pub(super) fn safe_name(name: &str) -> String {
    let folders: &[char] = if cfg!(windows) {
        &['/', '\\', ':']
    } else {
        &['/', '\\']
    };
    let base = name.rsplit(folders).next().unwrap_or_default();
    let base: String = base
        .chars()
        .filter(|c| {
            !matches!(
                c,
                '\u{200e}' | '\u{200f}' | '\u{061c}' | '\u{202a}'..='\u{202e}' | '\u{2066}'..='\u{2069}'
            )
        })
        .collect();
    let base = base.trim_start_matches('.').trim();
    if base.is_empty() {
        "attachment".to_string()
    } else {
        base.to_string()
    }
}

/// `dir/name`, or `dir/name (2)`, and so on, the first not there yet.
pub(super) fn unused(dir: &std::path::Path, name: &str) -> PathBuf {
    let path = dir.join(name);
    if !path.exists() {
        return path;
    }
    let (stem, ext) = match name.rsplit_once('.') {
        Some((s, e)) if !s.is_empty() => (s, format!(".{e}")),
        _ => (name, String::new()),
    };
    (2..)
        .map(|n| dir.join(format!("{stem} ({n}){ext}")))
        .find(|p| !p.exists())
        .expect("a name not taken")
}

impl App {
    /// Show the message read in the browser: its HTML, or its text.
    pub(super) fn open_in_browser(&mut self) {
        let Some(m) = self.message().cloned() else {
            self.status = "nothing to open: ⏎ reads the message picked".to_string();
            return;
        };
        let done = write_page(&m).and_then(|path| open(&path).map(|_| path));
        self.status = match done {
            Ok(path) => format!("opened {}", path.display()),
            Err(e) => format!("cannot open it: {e:#}"),
        };
    }

    /// Present the message read with ohp, as its HTML, its own images in
    /// it, or its text: ohp takes the terminal until it quits.
    pub(super) fn present(&mut self) {
        let Some(m) = self.message().cloned() else {
            self.status = "nothing to present: ⏎ reads the message picked".to_string();
            return;
        };
        match write_page(&m) {
            Ok(path) => self.launch = Some(Launch::Ohp(path)),
            Err(e) => self.status = format!("cannot present it: {e:#}"),
        }
    }

    /// List the attachments and links of the message read, to open.
    pub(super) fn list_files(&mut self) {
        let Some(m) = self.message().cloned() else {
            self.status = "nothing to list: ⏎ reads the message picked".to_string();
            return;
        };
        let mut items: Vec<Item> = vec![Item::Page, Item::Present];
        items.extend(m.attachments.iter().enumerate().map(|(at, a)| Item::File {
            at,
            name: a.name.clone(),
            mime: a.mime.clone(),
            size: a.size,
        }));
        items.extend(m.links.iter().cloned().map(Item::Link));
        // On its first file, if it has one; else on the message itself.
        let at = if items.len() > 2 { 2 } else { 0 };
        self.dialog = Dialog::Files(Files {
            items,
            at,
            note: None,
            saving: false,
        });
    }

    pub(super) fn files_key(&mut self, key: KeyEvent) {
        let Some(files) = self.dialog.files_mut() else {
            return;
        };
        if files.saving {
            return self.save_list_key(key);
        }
        match (keymap::act(key), key.code) {
            (Some(Act::Down), _) => self.dialog.step(1),
            (Some(Act::Up), _) => self.dialog.step(-1),
            (Some(Act::Open), _) | (_, KeyCode::Char('o')) => self.open_item(),
            (_, KeyCode::Char('s')) => {
                files.note = match files.items.get(files.at) {
                    Some(Item::File { .. }) => return self.pick_save_dir(false),
                    Some(Item::Link(_)) => Some("a link is opened, not saved: ⏎".to_string()),
                    _ => Some("only its files are saved: S saves them all".to_string()),
                };
            }
            (_, KeyCode::Char('S')) => {
                if !files.items.iter().any(|i| matches!(i, Item::File { .. })) {
                    files.note = Some("no attachments".to_string());
                } else {
                    self.pick_save_dir(true);
                }
            }
            (_, KeyCode::Char('y')) => {
                if let Some(Item::Link(url)) = files.items.get(files.at) {
                    self.launch = Some(Launch::Copy(url.clone()));
                }
            }
            _ => self.dialog = Dialog::Closed,
        }
    }

    /// Open the attachment or link picked: in ohp, a PDF or markdown, a
    /// linked PDF downloaded first; else by the system, or in the browser.
    pub(super) fn open_item(&mut self) {
        let m = self.message().cloned();
        let (Some(files), Some(m)) = (self.dialog.files_mut(), m) else {
            return;
        };
        match files.items.get(files.at).cloned() {
            Some(Item::Page) => {
                self.dialog = Dialog::Closed;
                self.open_in_browser();
            }
            Some(Item::Present) => {
                self.dialog = Dialog::Closed;
                self.present();
            }
            Some(Item::File { at, name, .. }) => {
                let Some((_, bytes)) = mail::attachments(&m.raw).into_iter().nth(at) else {
                    return;
                };
                match keep_file(m.uid, &name, &bytes) {
                    Ok(path) if ohp_reads(&name) => self.launch = Some(Launch::Ohp(path)),
                    Ok(path) => self.launch = Some(Launch::Open(path)),
                    Err(e) => files.note = Some(format!("cannot keep {name}: {e:#}")),
                }
            }
            Some(Item::Link(url)) => match crate::net::pdf_of(&url) {
                Some(pdf) => {
                    files.note = Some(format!("downloading {pdf}…"));
                    let (replies, uid) = (self.replies_to.clone(), m.uid);
                    self.pending += 1;
                    std::thread::spawn(move || {
                        let got = crate::net::pdf(&pdf)
                            .and_then(|(name, bytes)| keep_file(uid, &name, &bytes))
                            .map_err(worker::text);
                        let _ = replies.send(Reply {
                            account: usize::MAX,
                            answer: Answer::Fetched(got),
                        });
                    });
                }
                None => self.launch = Some(Launch::Browse(url)),
            },
            None => {}
        }
    }
}
