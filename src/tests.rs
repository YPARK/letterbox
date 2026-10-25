use crate::app::{self, App, Body, Focus};
use crate::config::{Account, Security};
use crate::demo::Demo;
use crate::imap::Imap;
use crate::mail::{self, Folder, Role};
use crate::secret::Keeper;
use crate::store::{Flag, Removal, Store};
use crate::ui;
use std::io::{BufRead, BufReader, Read, Write};
use std::net::{TcpListener, TcpStream};
use std::path::PathBuf;
use std::sync::{Arc, Mutex};
use std::time::Duration;

// ---- an IMAP server, just enough of one, on this machine ----

#[derive(Clone)]
struct Letter {
    uid: u32,
    flags: Vec<String>,
    raw: Vec<u8>,
    /// When it last changed, as CONDSTORE counts.
    modseq: u64,
    /// Gmail's labels on it, and its ID there.
    labels: Vec<String>,
    msgid: u64,
}

impl Letter {
    fn new(uid: u32, flags: &[&str], raw: Vec<u8>) -> Letter {
        Letter {
            uid,
            flags: flags.iter().map(|f| f.to_string()).collect(),
            raw,
            modseq: 1,
            labels: Vec::new(),
            msgid: 0,
        }
    }

    fn has(&self, flag: &str) -> bool {
        self.flags.iter().any(|f| f.eq_ignore_ascii_case(flag))
    }
}

struct Mailbox {
    name: String,
    attributes: &'static str,
    letters: Vec<Letter>,
    validity: u32,
    /// The UID the next message will have.
    next: u32,
}

impl Mailbox {
    fn new(name: &str, attributes: &'static str, letters: Vec<Letter>) -> Mailbox {
        let next = letters.iter().map(|l| l.uid + 1).max().unwrap_or(1);
        Mailbox {
            name: name.to_string(),
            attributes,
            letters,
            validity: 7,
            next,
        }
    }

    fn push(&mut self, letter: Letter) {
        self.next = self.next.max(letter.uid + 1);
        self.letters.push(letter);
    }
}

/// How the fake server keeps labels, as servers do.
#[derive(Clone, Copy, Debug, PartialEq)]
enum Mode {
    /// In folders alone, as Microsoft's: no keywords, no CONDSTORE.
    Folders,
    /// In keywords of any name, with CONDSTORE, as Dovecot's and Fastmail's.
    Keywords,
    /// As Gmail's labels, with its IDs, and CONDSTORE.
    Gmail,
}

/// The server's mail, how it keeps labels, its mod-sequence, and the
/// commands it was sent.
struct Fake {
    boxes: Vec<Mailbox>,
    mode: Mode,
    modseq: u64,
    said: Vec<String>,
}

impl std::ops::Deref for Fake {
    type Target = Vec<Mailbox>;
    fn deref(&self) -> &Vec<Mailbox> {
        &self.boxes
    }
}

impl std::ops::DerefMut for Fake {
    fn deref_mut(&mut self) -> &mut Vec<Mailbox> {
        &mut self.boxes
    }
}

impl Fake {
    /// The next mod-sequence, for a change made now.
    fn tick(&mut self) -> u64 {
        self.modseq += 1;
        self.modseq
    }
}

type Boxes = Arc<Mutex<Fake>>;

fn raw(from: &str, subject: &str, extra: &str, body: &str) -> Vec<u8> {
    let id: String = subject
        .chars()
        .filter(|c| c.is_ascii_alphanumeric())
        .take(40)
        .collect();
    format!(
        "From: {from}\r\nTo: me@example.com\r\nSubject: {subject}\r\n\
         Date: Tue, 20 Oct 2026 10:00:00 +0000\r\nMessage-ID: <{id}@example>\r\n\
         MIME-Version: 1.0\r\n{extra}\r\n{body}"
    )
    .into_bytes()
}

fn fake(boxes: Vec<Mailbox>, mode: Mode) -> Boxes {
    Arc::new(Mutex::new(Fake {
        boxes,
        mode,
        modseq: 1,
        said: Vec::new(),
    }))
}

fn boxes() -> Boxes {
    boxes_in(Mode::Folders)
}

fn boxes_in(mode: Mode) -> Boxes {
    let inbox = vec![
        Letter::new(
            11,
            &["\\Seen"],
            raw(
                "Rosalind Franklin <rf@kings.example>",
                "Photo 51",
                "Content-Type: text/plain; charset=utf-8\r\n",
                "The B form, at last.\r\n",
            ),
        ),
        Letter::new(
            12,
            &["\\Seen", "\\Flagged"],
            raw(
                "Alan Turing <alan@bletchley.example>",
                "=?utf-8?q?Computable_numbers_=E2=80=94_the_paper?=",
                "Content-Type: multipart/mixed; boundary=\"m1\"\r\n",
                "--m1\r\nContent-Type: text/plain; charset=utf-8\r\n\r\n\
                 > what you asked\r\nThe paper is attached.\r\n\r\n--m1\r\n\
                 Content-Type: text/plain; name=\"states.txt\"\r\n\
                 Content-Disposition: attachment; filename=\"../states.txt\"\r\n\r\n\
                 b None P0 c\r\n--m1--\r\n",
            ),
        ),
        Letter::new(
            14,
            &[],
            raw(
                "=?utf-8?q?Emmy_N=C3=B6ther?= <noether@goettingen.example>",
                "Symmetry\x1b[2J",
                "Content-Type: text/plain; charset=utf-8\r\n",
                "Every symmetry has its conserved quantity.\x1b]0;owned\x07\r\n",
            ),
        ),
    ];
    let mut all = vec![
        Mailbox::new("INBOX", "\\HasNoChildren", inbox),
        Mailbox::new("Projects", "\\HasChildren", Vec::new()),
        Mailbox::new("Projects/Grant", "\\HasNoChildren", Vec::new()),
        Mailbox::new("Deleted Items", "\\HasNoChildren", Vec::new()),
        Mailbox::new("Sent Items", "\\HasNoChildren \\Sent", Vec::new()),
        Mailbox::new("Entw&APw-rfe", "\\HasNoChildren", Vec::new()),
        Mailbox::new("Drafts", "\\HasNoChildren \\Drafts", Vec::new()),
    ];
    if mode == Mode::Keywords {
        all.push(Mailbox::new(
            "Archive",
            "\\HasNoChildren \\Archive",
            Vec::new(),
        ));
    }
    fake(all, mode)
}

/// Gmail's folders: the Inbox, and in [Gmail], All Mail, which holds every
/// message with its labels, and the rest; a label of the user's, as a
/// folder too.
fn gmail_boxes() -> Boxes {
    let letter = |uid: u32, flags: &[&str], labels: &[&str], from: &str, subject: &str| {
        let mut l = Letter::new(
            uid,
            flags,
            raw(
                from,
                subject,
                "Content-Type: text/plain; charset=utf-8\r\n",
                "Hello.\r\n",
            ),
        );
        l.labels = labels.iter().map(|s| s.to_string()).collect();
        l.msgid = 9000 + u64::from(uid);
        l
    };
    let all = vec![
        letter(
            21,
            &["\\Seen"],
            &["\\Inbox", "Projects/Grant"],
            "Mira Okafor <mira.okafor@uni.example>",
            "Aims page, third pass",
        ),
        letter(
            22,
            &[],
            &["\\Inbox", "\\Important"],
            "Research Office <grants@uni.example>",
            "R01 resubmission",
        ),
        letter(
            23,
            &["\\Seen"],
            &["\\Sent"],
            "Me <me@example.com>",
            "Re: Aims page",
        ),
    ];
    let inbox: Vec<Letter> = all
        .iter()
        .filter(|l| l.labels.iter().any(|x| x == "\\Inbox"))
        .map(|l| Letter {
            uid: l.uid - 20,
            ..l.clone()
        })
        .collect();
    fake(
        vec![
            Mailbox::new("INBOX", "\\HasNoChildren", inbox),
            Mailbox::new("[Gmail]", "\\HasChildren \\Noselect", Vec::new()),
            Mailbox::new("[Gmail]/All Mail", "\\HasNoChildren \\All", all),
            Mailbox::new("[Gmail]/Sent Mail", "\\HasNoChildren \\Sent", Vec::new()),
            Mailbox::new("[Gmail]/Drafts", "\\HasNoChildren \\Drafts", Vec::new()),
            Mailbox::new("[Gmail]/Spam", "\\HasNoChildren \\Junk", Vec::new()),
            Mailbox::new("[Gmail]/Trash", "\\HasNoChildren \\Trash", Vec::new()),
            Mailbox::new("Projects", "\\HasChildren", Vec::new()),
            Mailbox::new("Projects/Grant", "\\HasNoChildren", Vec::new()),
        ],
        Mode::Gmail,
    )
}

/// The IMAP strings in `s`, quoted or not, unquoted.
fn words(s: &str) -> Vec<String> {
    let mut out = Vec::new();
    let mut chars = s.chars().peekable();
    while let Some(&c) = chars.peek() {
        if c == ' ' {
            chars.next();
        } else if c == '"' {
            chars.next();
            let mut w = String::new();
            while let Some(c) = chars.next() {
                match c {
                    '\\' => w.extend(chars.next()),
                    '"' => break,
                    c => w.push(c),
                }
            }
            out.push(w);
        } else {
            let mut w = String::new();
            while let Some(&c) = chars.peek() {
                if c == ' ' {
                    break;
                }
                w.push(c);
                chars.next();
            }
            out.push(w);
        }
    }
    out
}

fn next_uid(boxes: &[Mailbox]) -> u32 {
    boxes.iter().map(|b| b.next).max().unwrap_or(1)
}

/// The byte count of an IMAP literal, as `{310}` at the end of a command.
fn literal_size(args: &str) -> usize {
    args.rsplit_once('{')
        .and_then(|(_, n)| n.trim_end_matches('}').parse().ok())
        .unwrap_or(0)
}

/// The ranges of an IMAP set, as `1:3,7` or `5:*`.
fn set(s: &str) -> Vec<(u32, u32)> {
    let n = |x: &str| {
        if x == "*" {
            u32::MAX
        } else {
            x.parse().unwrap_or(0)
        }
    };
    s.split(',')
        .map(|part| match part.split_once(':') {
            Some((a, b)) => (n(a).min(n(b)), n(a).max(n(b))),
            None => (n(part), n(part)),
        })
        .collect()
}

fn within(ranges: &[(u32, u32)], n: u32) -> bool {
    ranges.iter().any(|&(a, b)| a <= n && n <= b)
}

fn fetched(seq: usize, l: &Letter, items: &str, changed: bool) -> Vec<u8> {
    let mut out = format!("* {seq} FETCH (UID {}", l.uid).into_bytes();
    if items.contains("X-GM-MSGID") {
        out.extend(format!(" X-GM-MSGID {} X-GM-THRID {}", l.msgid, l.msgid).bytes());
    }
    if items.contains("X-GM-LABELS") {
        let labels: Vec<String> = l
            .labels
            .iter()
            .map(|x| {
                if x.starts_with('\\') {
                    x.clone()
                } else {
                    format!("\"{x}\"")
                }
            })
            .collect();
        out.extend(format!(" X-GM-LABELS ({})", labels.join(" ")).bytes());
    }
    if changed {
        out.extend(format!(" MODSEQ ({})", l.modseq).bytes());
    }
    if items.contains("BODY.PEEK[]") {
        out.extend(format!(" BODY[] {{{}}}\r\n", l.raw.len()).bytes());
        out.extend(&l.raw);
    } else if items.contains("BODY") {
        let split = l
            .raw
            .windows(4)
            .position(|w| w == b"\r\n\r\n")
            .map_or(l.raw.len(), |p| p + 4);
        let (header, text) = l.raw.split_at(split);
        let text = &text[..text.len().min(2048)];
        let fields = items
            .split("HEADER.FIELDS ")
            .nth(1)
            .and_then(|s| s.split(']').next())
            .unwrap_or("()");
        out.extend(
            format!(
                " FLAGS ({}) RFC822.SIZE {} BODY[HEADER.FIELDS {fields}] {{{}}}\r\n",
                l.flags.join(" "),
                l.raw.len(),
                header.len()
            )
            .bytes(),
        );
        out.extend(header);
        out.extend(format!(" BODY[TEXT]<0> {{{}}}\r\n", text.len()).bytes());
        out.extend(text);
    } else {
        out.extend(format!(" FLAGS ({})", l.flags.join(" ")).bytes());
    }
    out.extend(b")\r\n");
    out
}

/// The flags or labels a `STORE` names, in its parentheses.
fn listed(s: &str) -> Vec<String> {
    words(s.trim().trim_start_matches('(').trim_end_matches(')'))
}

fn serve(tcp: TcpStream, shared: Boxes) {
    let mut out = tcp.try_clone().unwrap();
    let mut lines = BufReader::new(tcp);
    let mut selected: Option<usize> = None;
    let mut condstore = false;
    out.write_all(b"* OK [CAPABILITY IMAP4rev1] fake ready\r\n")
        .unwrap();
    let mut line = String::new();
    loop {
        line.clear();
        if lines.read_line(&mut line).unwrap_or(0) == 0 {
            return;
        }
        let line = line.trim_end();
        let (tag, rest) = line.split_once(' ').unwrap_or((line, ""));
        let (command, args) = rest.split_once(' ').unwrap_or((rest, ""));
        let mut reply: Vec<u8> = Vec::new();
        let mut boxes = shared.lock().unwrap();
        boxes.said.push(rest.to_string());
        let mode = boxes.mode;
        let mut done = format!("{tag} OK done\r\n");
        match command.to_uppercase().as_str() {
            "LOGIN" => {
                if words(args) != ["me", "secret"] {
                    done = format!("{tag} NO [AUTHENTICATIONFAILED] wrong\r\n");
                }
            }
            "LIST" => {
                for b in boxes.iter() {
                    reply.extend(
                        format!("* LIST ({}) \"/\" \"{}\"\r\n", b.attributes, b.name).bytes(),
                    );
                }
            }
            "ENABLE" if mode != Mode::Folders => {
                condstore = true;
                reply.extend(b"* ENABLED CONDSTORE\r\n");
            }
            "SELECT" | "EXAMINE" => {
                let name = &words(args)[0];
                match boxes.iter().position(|b| b.name == *name) {
                    Some(i) => {
                        selected = Some(i);
                        let b = &boxes[i];
                        let keywords = if mode == Mode::Folders { "" } else { " \\*" };
                        reply.extend(
                            format!(
                                "* FLAGS (\\Seen \\Flagged)\r\n* {} EXISTS\r\n* 0 RECENT\r\n\
                                 * OK [UIDVALIDITY {}] ok\r\n* OK [UIDNEXT {}] ok\r\n\
                                 * OK [PERMANENTFLAGS (\\Seen \\Flagged \\Deleted \
                                 \\Answered{keywords})] ok\r\n",
                                b.letters.len(),
                                b.validity,
                                b.next,
                            )
                            .bytes(),
                        );
                        if condstore {
                            let highest = b.letters.iter().map(|l| l.modseq).max().unwrap_or(1);
                            reply.extend(format!("* OK [HIGHESTMODSEQ {highest}] ok\r\n").bytes());
                        }
                        done = format!("{tag} OK [READ-WRITE] selected\r\n");
                    }
                    None => done = format!("{tag} NO no such folder\r\n"),
                }
            }
            "STATUS" => {
                let w = words(args);
                let b = boxes.iter().find(|b| b.name == w[0]).unwrap();
                let asked = args.to_uppercase();
                let mut items = Vec::new();
                if asked.contains("MESSAGES") {
                    items.push(format!("MESSAGES {}", b.letters.len()));
                }
                if asked.contains("UNSEEN") {
                    let unseen = b.letters.iter().filter(|l| !l.has("\\Seen")).count();
                    items.push(format!("UNSEEN {unseen}"));
                }
                if asked.contains("UIDNEXT") {
                    items.push(format!("UIDNEXT {}", b.next));
                }
                if asked.contains("UIDVALIDITY") {
                    items.push(format!("UIDVALIDITY {}", b.validity));
                }
                if asked.contains("HIGHESTMODSEQ") {
                    let highest = b.letters.iter().map(|l| l.modseq).max().unwrap_or(1);
                    items.push(format!("HIGHESTMODSEQ {highest}"));
                }
                reply.extend(format!("* STATUS \"{}\" ({})\r\n", b.name, items.join(" ")).bytes());
            }
            "FETCH" => {
                let (seqs, items) = args.split_once(' ').unwrap();
                let b = &boxes[selected.unwrap()];
                let seqs = set(seqs);
                for (i, l) in b.letters.iter().enumerate() {
                    if within(&seqs, i as u32 + 1) {
                        reply.extend(fetched(i + 1, l, items, false));
                    }
                }
            }
            "UID" => {
                let (sub, args) = args.split_once(' ').unwrap();
                let sub = sub.to_uppercase();
                match sub.as_str() {
                    "FETCH" => {
                        let b = &boxes[selected.unwrap()];
                        let (uids, items) = args.split_once(' ').unwrap();
                        let since: Option<u64> = items
                            .split("(CHANGEDSINCE ")
                            .nth(1)
                            .and_then(|s| s.trim_end_matches(')').parse().ok());
                        let uids = set(uids);
                        for (i, l) in b.letters.iter().enumerate() {
                            if within(&uids, l.uid) && since.is_none_or(|m| l.modseq > m) {
                                reply.extend(fetched(i + 1, l, items, since.is_some()));
                            }
                        }
                    }
                    "STORE" => {
                        let seq = boxes.tick();
                        let (uids, rest) = args.split_once(' ').unwrap();
                        let (how, what) = rest.split_once(' ').unwrap();
                        let uids = set(uids);
                        let how = how.to_uppercase();
                        // Gmail shows each label as a folder, once it is given.
                        if how.starts_with("+X-GM-LABELS") {
                            for name in listed(what) {
                                if !name.starts_with('\\') && !boxes.iter().any(|b| b.name == name)
                                {
                                    boxes.push(Mailbox::new(&name, "\\HasNoChildren", Vec::new()));
                                }
                            }
                        }
                        let b = &mut boxes[selected.unwrap()];
                        for l in b.letters.iter_mut().filter(|l| within(&uids, l.uid)) {
                            let list = if how.contains("X-GM-LABELS") {
                                &mut l.labels
                            } else {
                                &mut l.flags
                            };
                            for name in listed(what) {
                                // A server without `\*` keeps no keyword it
                                // was not made with.
                                let keyword = !name.starts_with('\\');
                                if keyword && mode == Mode::Folders && !how.contains("X-GM") {
                                    continue;
                                }
                                list.retain(|f| !f.eq_ignore_ascii_case(&name));
                                if how.starts_with('+') {
                                    list.push(name);
                                }
                            }
                            l.modseq = seq;
                        }
                    }
                    "SEARCH" => {
                        let b = &boxes[selected.unwrap()];
                        let w = words(args);
                        let all = w.iter().any(|x| x.eq_ignore_ascii_case("ALL"))
                            && !w.iter().any(|x| {
                                x.eq_ignore_ascii_case("TEXT") || x.eq_ignore_ascii_case("HEADER")
                            });
                        let keyword = w
                            .iter()
                            .position(|x| x.eq_ignore_ascii_case("KEYWORD"))
                            .and_then(|i| w.get(i + 1));
                        let wanted = w.last().unwrap().to_lowercase();
                        let uids: Vec<String> = b
                            .letters
                            .iter()
                            .filter(|l| match keyword {
                                Some(k) => l.has(k),
                                None => {
                                    all || String::from_utf8_lossy(&l.raw)
                                        .to_lowercase()
                                        .contains(&wanted)
                                }
                            })
                            .map(|l| l.uid.to_string())
                            .collect();
                        reply.extend(format!("* SEARCH {}\r\n", uids.join(" ")).bytes());
                    }
                    "COPY" | "MOVE" => {
                        let w = words(args);
                        let uid: u32 = w[0].parse().unwrap_or(0);
                        let dest = w.get(1).cloned().unwrap_or_default();
                        let src = selected.unwrap();
                        let seq = boxes.tick();
                        if !boxes.iter().any(|b| b.name == dest) {
                            done = format!("{tag} NO no such folder\r\n");
                        } else if !boxes[src].letters.iter().any(|l| l.uid == uid) {
                            done = format!("{tag} NO no such message\r\n");
                        } else {
                            let i = boxes[src]
                                .letters
                                .iter()
                                .position(|l| l.uid == uid)
                                .unwrap();
                            let letter = if sub == "MOVE" {
                                boxes[src].letters.remove(i)
                            } else {
                                boxes[src].letters[i].clone()
                            };
                            let new_uid = next_uid(&boxes);
                            let at = boxes.iter().position(|b| b.name == dest).unwrap();
                            boxes[at].push(Letter {
                                uid: new_uid,
                                modseq: seq,
                                ..letter
                            });
                            let validity = boxes[at].validity;
                            done =
                                format!("{tag} OK [COPYUID {validity} {uid} {new_uid}] moved\r\n");
                        }
                    }
                    "EXPUNGE" => {
                        let b = &mut boxes[selected.unwrap()];
                        let uids = set(words(args).first().map(String::as_str).unwrap_or(""));
                        let mut seqs = Vec::new();
                        for (i, letter) in b.letters.iter().enumerate() {
                            if within(&uids, letter.uid) && letter.has("\\Deleted") {
                                seqs.push(i + 1);
                            }
                        }
                        b.letters
                            .retain(|l| !(within(&uids, l.uid) && l.has("\\Deleted")));
                        for seq in seqs.iter().rev() {
                            reply.extend(format!("* {seq} EXPUNGE\r\n").bytes());
                        }
                    }
                    _ => done = format!("{tag} BAD unknown\r\n"),
                }
            }
            "CREATE" => {
                let name = words(args).into_iter().next().unwrap_or_default();
                if boxes.iter().any(|b| b.name == name) {
                    done = format!("{tag} NO exists\r\n");
                } else {
                    boxes.push(Mailbox::new(&name, "\\HasNoChildren", Vec::new()));
                }
            }
            "RENAME" => {
                let w = words(args);
                let (from, to) = (w[0].clone(), w[1].clone());
                if !boxes.iter().any(|b| b.name == from) {
                    done = format!("{tag} NO no such folder\r\n");
                } else {
                    let under = format!("{from}/");
                    for b in boxes.iter_mut() {
                        if b.name == from {
                            b.name = to.clone();
                        } else if let Some(rest) = b.name.strip_prefix(&under) {
                            b.name = format!("{to}/{rest}");
                        }
                    }
                    // Gmail renames the label on every message it is on.
                    if mode == Mode::Gmail {
                        let seq = boxes.tick();
                        for b in boxes.iter_mut() {
                            for l in &mut b.letters {
                                for label in &mut l.labels {
                                    if *label == from {
                                        *label = to.clone();
                                        l.modseq = seq;
                                    }
                                }
                            }
                        }
                    }
                }
            }
            "APPEND" => {
                let size = literal_size(args);
                let name = words(args).into_iter().next().unwrap_or_default();
                out.write_all(b"+ go\r\n").unwrap();
                let mut body = vec![0u8; size];
                if lines.get_mut().read_exact(&mut body).is_err() {
                    return;
                }
                let mut tail = [0u8; 2];
                let _ = lines.get_mut().read_exact(&mut tail);
                let new_uid = next_uid(&boxes);
                let flags: &[&str] = if args.contains("\\Seen") {
                    &["\\Seen"]
                } else {
                    &[]
                };
                match boxes.iter_mut().find(|b| b.name == name) {
                    Some(b) => b.push(Letter::new(new_uid, flags, body)),
                    None => done = format!("{tag} NO no such folder\r\n"),
                }
                if !done.starts_with(&format!("{tag} NO")) {
                    done = format!("{tag} OK appended\r\n");
                }
            }
            "LOGOUT" => {
                let _ = out.write_all(format!("* BYE\r\n{tag} OK bye\r\n").as_bytes());
                return;
            }
            "AUTHENTICATE" => {
                // XOAUTH2: the token, after the server's `+`, in base64.
                drop(boxes);
                out.write_all(b"+ \r\n").unwrap();
                let mut answer = String::new();
                lines.read_line(&mut answer).unwrap();
                use base64::Engine;
                let said = base64::engine::general_purpose::STANDARD
                    .decode(answer.trim())
                    .unwrap_or_default();
                let said = String::from_utf8_lossy(&said);
                let good = said.starts_with("user=me\x01auth=Bearer access-")
                    && said.ends_with("\x01\x01");
                let done = if good {
                    format!("{tag} OK signed in\r\n")
                } else {
                    format!("{tag} NO [AUTHENTICATIONFAILED] bad token\r\n")
                };
                out.write_all(done.as_bytes()).unwrap();
                continue;
            }
            "CAPABILITY" => {
                let more = match mode {
                    Mode::Folders => "",
                    Mode::Keywords => " ENABLE CONDSTORE",
                    Mode::Gmail => " ENABLE CONDSTORE X-GM-EXT-1",
                };
                reply.extend(format!("* CAPABILITY IMAP4rev1 MOVE UIDPLUS{more}\r\n").bytes());
            }
            "NOOP" => {}
            _ => done = format!("{tag} BAD unknown\r\n"),
        }
        drop(boxes);
        reply.extend(done.bytes());
        if out.write_all(&reply).is_err() {
            return;
        }
    }
}

/// A server on this machine, and an account on it.
fn server() -> (Account, Boxes) {
    serve_boxes(boxes())
}

/// A server on this machine with the mail `boxes` holds, and an account
/// on it.
fn serve_boxes(boxes: Boxes) -> (Account, Boxes) {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let port = listener.local_addr().unwrap().port();
    let shared = boxes.clone();
    std::thread::spawn(move || {
        for tcp in listener.incoming().flatten() {
            let b = shared.clone();
            std::thread::spawn(move || serve(tcp, b));
        }
    });
    let account = Account {
        name: "Fake".to_string(),
        email: "me@example.com".to_string(),
        host: "127.0.0.1".to_string(),
        port: Some(port),
        user: Some("me".to_string()),
        security: Security::None,
        auth: crate::config::Auth::Password,
        tenant: None,
        client_id: None,
        keep: Keeper::Keyring,
        password_command: Some("echo secret".to_string()),
        password: None,
        missing: None,
        smtp_host: None,
        smtp_port: None,
        trash: None,
    };
    (account, boxes)
}

#[test]
fn imap_lists_folders_in_a_mail_apps_order() {
    let (account, _) = server();
    let folders = Imap::new(account).folders().unwrap();
    let shown: Vec<(&str, usize, Role)> = folders
        .iter()
        .map(|f| (f.name.as_str(), f.depth, f.role))
        .collect();
    assert_eq!(
        shown,
        [
            ("Inbox", 0, Role::Inbox),
            ("Drafts", 0, Role::Drafts),
            ("Sent Items", 0, Role::Sent),
            ("Deleted Items", 0, Role::Trash),
            ("Entwürfe", 0, Role::Other),
            ("Projects", 0, Role::Other),
            ("Grant", 1, Role::Other),
        ]
    );
    assert_eq!(folders[4].path, "Entw&APw-rfe");
}

#[test]
fn imap_pages_newest_first_and_reads_and_marks() {
    let (account, boxes) = server();
    let mut imap = Imap::new(account);
    assert_eq!(imap.counts("INBOX").unwrap(), (3, 1));

    let page = imap.page("INBOX", 0, 2).unwrap();
    assert_eq!(page.total, 3);
    let uids: Vec<u32> = page.summaries.iter().map(|s| s.uid).collect();
    assert_eq!(uids, [14, 12]);
    let emmy = &page.summaries[0];
    assert_eq!(emmy.from, "Emmy Nöther");
    assert_eq!(emmy.address, "noether@goettingen.example");
    assert!(!emmy.flags.seen);
    // An escape in a subject or a text never reaches the terminal.
    assert!(!emmy.subject.contains('\x1b'), "{:?}", emmy.subject);
    assert!(!emmy.preview.contains('\x1b') && !emmy.preview.contains('\x07'));
    let alan = &page.summaries[1];
    assert_eq!(alan.subject, "Computable numbers — the paper");
    assert!(alan.flags.flagged && alan.flags.seen);
    assert_eq!(alan.preview, "The paper is attached.");

    let older = imap.page("INBOX", 2, 2).unwrap();
    assert_eq!(
        older.summaries.iter().map(|s| s.uid).collect::<Vec<_>>(),
        [11]
    );
    assert!(imap.page("INBOX", 3, 2).unwrap().summaries.is_empty());

    let m = mail::message(12, imap.raw("INBOX", 12).unwrap()).unwrap();
    assert!(m.text.contains("The paper is attached."), "{}", m.text);
    assert_eq!(m.attachments.len(), 1);
    assert_eq!(m.attachments[0].name, "../states.txt");
    // Fetching it did not mark it read: only `set` does.
    assert_eq!(imap.counts("INBOX").unwrap(), (3, 1));
    imap.set("INBOX", 14, Flag::Seen, true).unwrap();
    assert_eq!(imap.counts("INBOX").unwrap(), (3, 0));
    imap.set("INBOX", 12, Flag::Flagged, false).unwrap();
    let flags = boxes.lock().unwrap()[0].letters[1].flags.clone();
    assert_eq!(flags, ["\\Seen"]);

    let found = imap.search("INBOX", "photo", 10).unwrap();
    assert_eq!(
        found.iter().map(|s| s.subject.as_str()).collect::<Vec<_>>(),
        ["Photo 51"]
    );
}

#[test]
fn imap_says_when_it_cannot_sign_in() {
    let (mut account, _) = server();
    account.password_command = Some("echo wrong".to_string());
    let e = format!("{:#}", Imap::new(account).folders().unwrap_err());
    assert!(e.contains("cannot sign in to 127.0.0.1 as me"), "{e}");
}

#[test]
fn a_password_command_that_fails_is_said() {
    let (mut account, _) = server();
    // As sh and Windows's cmd both read it.
    account.password_command = Some("echo no such entry 1>&2 && exit 1".to_string());
    let e = format!("{:#}", Imap::new(account).folders().unwrap_err());
    assert!(e.contains("no such entry"), "{e}");
}

#[test]
fn unencrypted_accounts_are_for_this_machine_only() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("accounts.toml");
    std::fs::write(
        &path,
        "[[account]]\nname = \"W\"\nemail = \"a@b.c\"\nhost = \"imap.b.c\"\nsecurity = \"none\"\n",
    )
    .unwrap();
    assert!(crate::config::load(&path).is_err());
    std::fs::write(
        &path,
        "[[account]]\nname = \"W\"\nemail = \"a@b.c\"\nhost = \"imap.b.c\"\n",
    )
    .unwrap();
    let accounts = crate::config::load(&path).unwrap();
    assert_eq!((accounts[0].user(), accounts[0].port()), ("a@b.c", 993));
    assert!(
        crate::config::load(&dir.path().join("none.toml"))
            .unwrap()
            .is_empty()
    );
}

// ---- what the map reads of a server ----

/// A message in `folder` of the fake server, with `flags`, from a list.
fn add_listed(boxes: &Boxes, folder: &str, flags: &[&str], subject: &str) -> u32 {
    let mut fake = boxes.lock().unwrap();
    let uid = next_uid(&fake);
    let b = fake.iter_mut().find(|b| b.name == folder).unwrap();
    b.push(Letter::new(
        uid,
        flags,
        raw(
            "GitHub <notifications@github.example>",
            subject,
            "Content-Type: text/plain; charset=utf-8\r\n\
             List-Id: letterbox <letterbox.YPARK.github.example>\r\n",
            "Merged into main.\r\n",
        ),
    ));
    uid
}

#[test]
fn a_keyword_server_says_its_folders_state_and_what_changed() {
    let (account, boxes) = serve_boxes(boxes_in(Mode::Keywords));
    let pr = add_listed(
        &boxes,
        "INBOX",
        &["\\Seen", "Grant", "$Junk"],
        "PR #38 merged",
    );
    let mut imap = Imap::new(account);
    assert_eq!(imap.kind().unwrap(), crate::store::Kind::Keywords);
    let status = imap.status("INBOX").unwrap();
    assert_eq!(
        (status.messages, status.validity, status.next),
        (4, 7, pr + 1)
    );
    let since = status.modseq.expect("CONDSTORE");
    assert_eq!(imap.uids("INBOX").unwrap(), [11, 12, 14, pr]);

    let got = imap.fetch_index("INBOX", &[12, pr]).unwrap();
    assert_eq!(got.iter().map(|i| i.uid).collect::<Vec<_>>(), [12, pr]);
    let listed = got.iter().find(|i| i.uid == pr).unwrap();
    assert!(listed.flags.seen);
    assert_eq!(
        listed.labels,
        ["Grant"],
        "keywords, but a mail program's own"
    );
    let record = crate::index::Record::of("me@127.0.0.1", "INBOX", 7, listed, 0);
    assert_eq!(record.list, "letterbox.ypark.github.example");
    assert_eq!(record.subject, "PR #38 merged");
    assert_eq!(record.key.id, "id:<PR38merged@example>");

    // A keyword given elsewhere is told by what changed since.
    assert!(imap.changed("INBOX", Some(since), &[]).unwrap().is_empty());
    {
        let mut fake = boxes.lock().unwrap();
        let seq = fake.tick();
        let l = &mut fake[0].letters[1];
        l.flags.push("Lab".into());
        l.modseq = seq;
    }
    let changed = imap.changed("INBOX", Some(since), &[]).unwrap();
    assert_eq!(changed.len(), 1);
    assert_eq!(
        (changed[0].uid, changed[0].labels.clone()),
        (12, vec!["Lab".to_string()])
    );
    assert!(changed[0].flags.flagged);
    let said = boxes.lock().unwrap().said.join("\n");
    assert!(said.contains("ENABLE CONDSTORE"), "{said}");
    assert!(said.contains("CHANGEDSINCE"), "{said}");
}

#[test]
fn a_server_of_folders_alone_is_read_by_its_flags() {
    let (account, _) = server();
    let mut imap = Imap::new(account);
    assert_eq!(imap.kind().unwrap(), crate::store::Kind::Folders);
    let status = imap.status("INBOX").unwrap();
    assert_eq!((status.validity, status.next, status.modseq), (7, 15, None));
    // No mod-sequences: the flags of the messages named are read instead.
    let changed = imap.changed("INBOX", Some(3), &[11, 14]).unwrap();
    let seen: Vec<(u32, bool)> = changed.iter().map(|c| (c.uid, c.flags.seen)).collect();
    assert_eq!(seen, [(11, true), (14, false)]);
}

#[test]
fn gmail_is_read_by_its_ids_and_labels_in_all_mail() {
    let (account, _) = serve_boxes(gmail_boxes());
    let mut imap = Imap::new(account);
    assert_eq!(imap.kind().unwrap(), crate::store::Kind::Gmail);
    let folders = imap.folders().unwrap();
    let all: Vec<&str> = folders
        .iter()
        .filter(|f| f.all)
        .map(|f| f.path.as_str())
        .collect();
    assert_eq!(all, ["[Gmail]/All Mail"]);
    let got = imap.fetch_index("[Gmail]/All Mail", &[21, 22, 23]).unwrap();
    assert_eq!(got.len(), 3);
    let aims = &got[0];
    assert_eq!((aims.gmail, aims.thread), (Some(9021), Some(9021)));
    assert_eq!(aims.labels, ["\\Inbox", "Projects/Grant"]);
    let record = crate::index::Record::of("me@127.0.0.1", "[Gmail]/All Mail", 7, aims, 0);
    assert_eq!(record.key.id, "gm:9021");
}

#[test]
fn a_set_of_uids_is_written_in_runs() {
    assert_eq!(crate::imap::uid_set(&[7, 1, 2, 3, 9, 10, 4]), "1:4,7,9:10");
    assert_eq!(crate::imap::uid_set(&[5]), "5");
    let (flags, keywords) = crate::imap::flags_named(["\\Seen", "Grant", "$Forwarded", "NonJunk"]);
    assert!(flags.seen && !flags.flagged);
    assert_eq!(keywords, ["Grant"]);
}

// ---- mail ----

#[test]
fn modified_utf7_names_are_decoded() {
    assert_eq!(mail::utf7_imap("Entw&APw-rfe"), "Entwürfe");
    assert_eq!(mail::utf7_imap("&ZeVnLIqe-"), "日本語");
    assert_eq!(mail::utf7_imap("Tom &- Jerry"), "Tom & Jerry");
    assert_eq!(mail::utf7_imap("broken&"), "broken&");
}

#[test]
fn escapes_from_a_server_never_reach_the_terminal() {
    // `&ABs-` is an escape, in modified UTF-7.
    let folder = Folder::new("&ABs-[2J&ABs-]0;x", Some("/"), None, true);
    assert!(!folder.name.contains('\u{1b}'), "{:?}", folder.name);
    let said = mail::clean_lines("NO \u{1b}[31mno\u{7}\nsecond line");
    assert_eq!(said, "NO  [31mno \nsecond line");
}

#[test]
fn gmails_folders_come_up_to_the_top() {
    let mut folders = vec![
        Folder::new("[Gmail]", Some("/"), None, false),
        Folder::new("[Gmail]/All Mail", Some("/"), Some(Role::Archive), true),
        Folder::new("[Gmail]/Sent Mail", Some("/"), Some(Role::Sent), true),
        Folder::new("[Gmail]/Starred", Some("/"), None, true),
        Folder::new("Zebra", Some("/"), None, true),
        Folder::new("INBOX", Some("/"), None, true),
        Folder::new("INBOX/Lab", Some("/"), None, true),
    ];
    mail::arrange(&mut folders);
    let names: Vec<(&str, usize)> = folders.iter().map(|f| (f.name.as_str(), f.depth)).collect();
    assert_eq!(
        names,
        [
            ("Inbox", 0),
            ("Lab", 1),
            ("Sent Mail", 0),
            ("All Mail", 0),
            ("[Gmail]", 0),
            ("Starred", 1),
            ("Zebra", 0),
        ]
    );
}

#[test]
fn html_only_mail_is_read_as_text() {
    let m = mail::message(
        1,
        raw(
            "a@b.c",
            "News",
            "Content-Type: text/html; charset=utf-8\r\n",
            "<html><body><p>Hello <b>there</b>.</p><p>Second.</p></body></html>",
        ),
    )
    .unwrap();
    assert!(m.text.contains("Hello there."), "{:?}", m.text);
    assert!(m.html.is_some());
}

#[test]
fn text_is_wrapped_to_the_width() {
    assert_eq!(
        ui::wrap("one two three four", 9),
        ["one two", "three", "four"]
    );
    assert_eq!(
        ui::wrap("  indented line here", 12),
        ["  indented", "line here"]
    );
    assert_eq!(ui::wrap("abcdefghijkl", 8), ["abcdefgh", "ijkl"]);
    assert_eq!(ui::wrap("a\n\nb", 8), ["a", "", "b"]);
}

#[test]
fn dates_are_shown_as_a_mail_app_shows_them() {
    let at: jiff::Timestamp = "2026-10-21T16:00:00Z".parse().unwrap();
    let now = at.to_zoned(jiff::tz::TimeZone::UTC);
    let ago = |hours: i64| at.as_second() - hours * 3600;
    assert_eq!(now.strftime("%A %H:%M").to_string(), "Wednesday 16:00");
    assert_eq!(ui::short_date(ago(2), &now), "2:00 PM");
    assert_eq!(ui::short_date(ago(20), &now), "Yesterday");
    assert_eq!(ui::short_date(ago(24 * 4), &now), "Saturday");
    assert_eq!(ui::short_date(ago(24 * 30), &now), "2026-09-21");
}

// ---- the app, on the demo's mail ----

fn demo() -> App {
    let mut app = App::new(
        vec![("Demo".to_string(), Box::new(Demo::new()) as Box<dyn Store>)],
        PathBuf::from("/nowhere/accounts.toml"),
    );
    // A ledger of this test's own, not the one letterbox keeps for itself.
    app.ledger = scratch_ledger();
    app.settle(Duration::from_secs(5));
    app
}

fn scratch_ledger() -> crate::trash::Ledger {
    static N: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
    let n = N.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
    let path =
        std::env::temp_dir().join(format!("letterbox-trash-{}-{n}.toml", std::process::id()));
    let _ = std::fs::remove_file(&path);
    crate::trash::Ledger::open(path)
}

fn press(app: &mut App, keys: &[&str]) {
    for key in keys.iter().flat_map(|k| app::keys(k)) {
        let _ = app::screen(app, 140, 40);
        app.key(key);
        app.settle(Duration::from_secs(5));
    }
}

/// The demo's Inbox: the six letters, and the lab's mail still to deal with;
/// and how many of them are unread.
const INBOX: usize = 33;
const UNREAD: u32 = 18;

fn unseen(app: &App) -> Option<u32> {
    app.folder().and_then(|f| f.unseen)
}

#[test]
fn the_inbox_is_listed_first() {
    let app = demo();
    assert_eq!(app.folder_name(), "Inbox");
    assert_eq!(app.shown.len(), INBOX);
    assert_eq!(app.shown[0].from, "Ada Lovelace");
    assert_eq!(unseen(&app), Some(UNREAD));
    // Listed, a message is not read, nor marked read.
    assert!(app.reading.is_none());
}

#[test]
fn reading_a_message_marks_it_read_and_u_unmarks_it() {
    let mut app = demo();
    press(&mut app, &["<enter>"]);
    assert_eq!(app.focus, Focus::Reader);
    let Some(Body::Ready(m)) = app.reading.as_ref().map(|r| &r.body) else {
        panic!("not read");
    };
    assert!(m.text.contains("sets out how"));
    assert!(app.shown[0].flags.seen);
    assert_eq!(unseen(&app), Some(UNREAD - 1));
    press(&mut app, &["u"]);
    assert!(!app.shown[0].flags.seen);
    assert_eq!(unseen(&app), Some(UNREAD));
    let screen = app::screen(&mut app, 140, 40);
    assert!(
        screen.contains("From  Ada Lovelace <ada@analytical.example>"),
        "{screen}"
    );
}

#[test]
fn n_reads_the_next_and_search_narrows() {
    let mut app = demo();
    press(&mut app, &["<enter>", "n"]);
    assert_eq!(
        app.message().unwrap().subject,
        "[letterbox] Issue #41: a map of the inbox"
    );
    press(&mut app, &["<esc>", "/", "photo 51"]);
    assert_eq!(app.shown.len(), 1);
    press(&mut app, &["<esc>"]);
    assert_eq!(app.shown.len(), INBOX);
    // What only the text has is found by the server, on ⏎.
    press(&mut app, &["/", "bernoulli numbers step", "<enter>"]);
    assert_eq!(app.shown.len(), 1, "{}", app.status);
    assert_eq!(app.shown[0].from, "Ada Lovelace");
}

#[test]
fn another_folder_is_listed_from_the_sidebar() {
    let mut app = demo();
    press(&mut app, &["h", "j", "j", "<enter>"]);
    assert_eq!(app.folder_name(), "Sent");
    assert_eq!(app.shown[0].subject, "Re: Photo 51");
    assert_eq!(app.focus, Focus::List);
}

#[test]
fn a_narrow_screen_shows_one_pane_at_a_time() {
    let mut app = demo();
    let list = app::screen(&mut app, 60, 20);
    assert!(
        list.contains("Ada Lovelace") && !list.contains("DEMO"),
        "{list}"
    );
    press(&mut app, &["<enter>"]);
    let read = app::screen(&mut app, 60, 20);
    assert!(
        read.contains("From  Ada Lovelace") && !read.contains("Grace"),
        "{read}"
    );
    press(&mut app, &["<esc>", "h"]);
    let side = app::screen(&mut app, 60, 20);
    assert!(side.contains("DEMO") && side.contains("Drafts"), "{side}");
}

#[test]
fn attachments_are_saved_under_a_name_of_their_own() {
    let dir = tempfile::tempdir().unwrap();
    // SAFETY: tests that read it are this one alone.
    unsafe { std::env::set_var("LETTERBOX_DOWNLOADS", dir.path()) };
    let mut app = demo();
    let turing = app
        .shown
        .iter()
        .position(|m| m.from == "Alan Turing")
        .unwrap();
    app.list.select(Some(turing));
    press(&mut app, &["<enter>"]);
    assert_eq!(
        app.message().unwrap().subject,
        "Computable numbers: the paper, and the figures"
    );
    // All of them, twice, from what o opens: S.
    press(&mut app, &["o", "S", "o", "S"]);
    assert!(dir.path().join("states.txt").is_file());
    assert!(
        dir.path().join("states (2).txt").is_file(),
        "{}",
        app.status
    );
    assert!(dir.path().join("computable-numbers (2).pdf").is_file());

    // One of them, from the list of its files and links.
    let cache = tempfile::tempdir().unwrap();
    unsafe { std::env::set_var("XDG_CACHE_HOME", cache.path()) };
    press(&mut app, &["o"]);
    let files = app.dialog.files().expect("listed");
    assert_eq!(
        files.items.len(),
        3,
        "the message, two attachments, no links"
    );
    assert_eq!(files.at, 1, "begun on the first file");
    press(&mut app, &["j", "s"]);
    assert!(dir.path().join("computable-numbers (3).pdf").is_file());

    // ⏎ hands a PDF to ohp, and another file to the system; the mouse too.
    press(&mut app, &["<enter>"]);
    let Some(app::Launch::Ohp(pdf)) = app.launch.take() else {
        panic!("not for ohp: {:?}", app.launch);
    };
    assert!(std::fs::read(&pdf).unwrap().starts_with(b"%PDF"));
    assert!(pdf.starts_with(cache.path()));
    click_at(&mut app, app::Target::Item(1));
    let Some(app::Launch::Open(text)) = app.launch.take() else {
        panic!("not for the system: {:?}", app.launch);
    };
    let uid = app.message().unwrap().uid;
    assert_eq!(
        text.file_name().unwrap().to_string_lossy(),
        format!("{uid}-states.txt")
    );
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let mode = std::fs::metadata(&text).unwrap().permissions().mode();
        assert_eq!(mode & 0o777, 0o600, "only yours");
    }
    press(&mut app, &["<esc>"]);
    assert!(app.dialog.files().is_none());
    unsafe { std::env::remove_var("XDG_CACHE_HOME") };
}

#[test]
fn links_are_found_in_text_and_html() {
    let text = "See https://arxiv.org/abs/2412.08821, and (https://example.org/a?b=1&c=2).\n\
                Again https://arxiv.org/abs/2412.08821 or mailto:x@y.z";
    let html =
        "<a href=\"https://example.org/x.pdf\">x</a> <a href='https://example.org/a?b=1&amp;c=2'>";
    assert_eq!(
        mail::links(text, Some(html)),
        [
            "https://arxiv.org/abs/2412.08821",
            "https://example.org/a?b=1&c=2",
            "https://example.org/x.pdf",
        ]
    );
    use crate::net::pdf_of;
    assert_eq!(
        pdf_of("https://arxiv.org/abs/2412.08821v2").as_deref(),
        Some("https://arxiv.org/pdf/2412.08821v2")
    );
    assert_eq!(
        pdf_of("https://example.org/paper.PDF?dl=1").as_deref(),
        Some("https://example.org/paper.PDF?dl=1")
    );
    assert_eq!(pdf_of("https://example.org/page"), None);
}

#[test]
fn a_linked_pdf_is_downloaded_and_a_page_is_not_taken_for_one() {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let at = format!("http://127.0.0.1:{}", listener.local_addr().unwrap().port());
    std::thread::spawn(move || {
        for mut tcp in listener.incoming().flatten() {
            let mut first = String::new();
            BufReader::new(tcp.try_clone().unwrap())
                .read_line(&mut first)
                .unwrap();
            let body: &[u8] = if first.contains("/paper.pdf") {
                b"%PDF-1.4 a paper"
            } else {
                b"<html>sign in first</html>"
            };
            let _ = write!(
                tcp,
                "HTTP/1.1 200 OK\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
                body.len()
            );
            let _ = tcp.write_all(body);
        }
    });
    let (name, bytes) = crate::net::pdf(&format!("{at}/papers/paper.pdf")).unwrap();
    assert_eq!(
        (name.as_str(), &bytes[..8]),
        ("paper.pdf", &b"%PDF-1.4"[..])
    );
    let e = crate::net::pdf(&format!("{at}/login")).unwrap_err();
    assert!(e.to_string().contains("gave no PDF"), "{e}");
    assert!(crate::net::pdf("file:///etc/passwd").is_err());
}

#[test]
fn the_screen_fits_any_size_and_wraps_what_is_read() {
    const LINK: &str =
        "https://example.org/papers/lovelace/notes-on-the-analytical-engine-1843.pdf";
    for keys in [
        &[][..],
        &["<enter>"],
        &["h"],
        &["?"],
        &["/", "a long query typed in"],
    ] {
        for width in [16, 24, 33, 50, 69, 70, 90, 119, 120, 160] {
            for height in [6, 9, 16, 48] {
                let mut app = demo();
                for key in keys.iter().flat_map(|k| app::keys(k)) {
                    let _ = app::screen(&mut app, width, height);
                    app.key(key);
                    app.settle(Duration::from_secs(5));
                }
                let screen = app::screen(&mut app, width, height);
                assert_eq!(screen.lines().count(), height as usize);
            }
        }
    }
    // Read whole, at any width, the message is all there, the link too.
    for width in [24, 40, 78, 140] {
        let mut app = demo();
        press(&mut app, &["<enter>"]);
        let mut text = String::new();
        let screen = app::screen(&mut app, width, 200);
        for line in screen.lines().skip(1) {
            if let Some(l) = line.split("│ ").nth(1) {
                text.push_str(l.trim_end());
                text.push(' ');
            }
        }
        assert!(!text.contains('…'), "{width}: {text}");
        let joined: String = text.split_whitespace().collect();
        assert!(joined.contains(LINK), "{width}: {text}");
        assert!(
            joined.contains("ifyouwouldratherreaditthere."),
            "{width}: {text}"
        );
    }
}

#[test]
fn long_words_break_after_a_slash_or_a_dot() {
    let lines = ui::wrap("see https://example.org/papers/lovelace/notes.pdf now", 24);
    assert_eq!(
        lines,
        [
            "see",
            "https://example.org/",
            "papers/lovelace/",
            "notes.pdf now"
        ]
    );
    for l in &lines {
        assert!(unicode_width::UnicodeWidthStr::width(l.as_str()) <= 24);
    }
}

// ---- adding and removing accounts ----

/// Where a test keeps a password: nowhere, the system's store left alone.
fn keep_in_test(_: Keeper, _: &str, _: &str, _: &str) -> anyhow::Result<()> {
    Ok(())
}

fn empty_app(config: PathBuf) -> App {
    microsoft();
    let mut app = App::new(Vec::new(), config);
    app.keep = keep_in_test;
    app.keepers = || vec![Keeper::Keyring, Keeper::Vault];
    app.browse = |_| {};
    app
}

/// The form filled in for the fake server, `password` typed last.
fn fill_form(app: &mut App, port: u16, password: &str) {
    let port = port.to_string();
    press(
        app,
        &[
            "A",
            "me@example.com",
            "<tab>",
            "<tab>",
            "<c-u>",
            "127.0.0.1",
            "<tab>",
            "<c-u>",
            &port,
            "<tab>",
            "<c-u>",
            "me",
            "<tab>",
            "<tab>",
            password,
            "<enter>",
        ],
    );
}

#[test]
fn an_account_is_added_from_the_tui_once_it_signs_in_and_removed() {
    let (account, _) = server();
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("letterbox/accounts.toml");
    let mut app = empty_app(path.clone());
    assert!(app::screen(&mut app, 120, 30).contains("Press A to add one"));
    press(&mut app, &["A", "me@example.com"]);
    let form = app.dialog.form().unwrap();
    assert_eq!(form.fields[1], "example.com");
    assert_eq!(form.fields[2], "imap.example.com");
    assert_eq!(form.fields[4], "me@example.com");
    press(&mut app, &["<esc>"]);
    assert!(app.dialog.form().is_none());

    fill_form(&mut app, account.port.unwrap(), "secret");
    assert!(
        app.dialog.form().is_none(),
        "{:?}",
        app.dialog.form().and_then(|f| f.note.clone())
    );
    assert_eq!(app.status, "added example.com");
    assert_eq!(app.folder_name(), "Inbox");
    assert_eq!(app.shown.len(), 3);
    let file = std::fs::read_to_string(&path).unwrap();
    assert!(file.contains("host = \"127.0.0.1\""), "{file}");
    assert!(!file.contains("secret") && !file.contains("keep"), "{file}");
    assert!(file.contains("user = \"me\""), "{file}");
    assert!(file.contains("security = \"none\""), "{file}");
    // As read again, it is the account added.
    let accounts = crate::config::load(&path).unwrap();
    assert_eq!(accounts[0].port, account.port);

    press(&mut app, &["h", "D", "n"]);
    assert_eq!(crate::config::load(&path).unwrap().len(), 1);
    press(&mut app, &["D", "y"]);
    assert!(crate::config::load(&path).unwrap().is_empty());
    assert!(app.listing.is_none());
    assert!(app::screen(&mut app, 120, 30).contains("No accounts yet."));
}

#[test]
fn an_account_that_cannot_sign_in_is_not_added() {
    let (account, _) = server();
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("accounts.toml");
    let mut app = empty_app(path.clone());
    fill_form(&mut app, account.port.unwrap(), "wrong");
    let form = app.dialog.form().expect("the form stays open");
    assert_eq!(form.failed, Some(app::Failure::Refused));
    assert!(
        form.note.as_ref().unwrap().contains("cannot sign in"),
        "{:?}",
        form.note
    );
    assert!(!path.exists());
    let screen = app::screen(&mut app, 100, 30);
    assert!(
        screen.contains("Add account") && screen.contains("•••••▏"),
        "{screen}"
    );
}

/// The form filled in for the fake server, as for a Gmail address, up to
/// the password.
fn fill_gmail_form(app: &mut App, port: u16) {
    let port = port.to_string();
    press(
        app,
        &[
            "A",
            "me@gmail.com",
            "<tab>",
            "<tab>",
            "<c-u>",
            "127.0.0.1",
            "<tab>",
            "<c-u>",
            &port,
            "<tab>",
            "<c-u>",
            "me",
            "<tab>",
            "<tab>",
        ],
    );
}

#[test]
fn a_failed_sign_in_still_says_where_an_app_password_is_made() {
    let (account, _) = server();
    let dir = tempfile::tempdir().unwrap();
    let mut app = empty_app(dir.path().join("accounts.toml"));
    fill_gmail_form(&mut app, account.port.unwrap());
    press(&mut app, &["wrong", "<enter>"]);
    let form = app.dialog.form().expect("the form stays open");
    assert_eq!(form.failed, Some(app::Failure::Refused));
    let screen = app::screen(&mut app, 100, 40);
    assert!(screen.contains("cannot sign in"), "{screen}");
    assert!(screen.contains("apppasswords"), "{screen}");
    let screen = app::screen(&mut app, 80, 24);
    assert!(screen.contains("Sign in and add"), "{screen}");
}

#[test]
fn a_mistake_in_the_form_is_said_without_the_providers_note() {
    let dir = tempfile::tempdir().unwrap();
    let mut app = empty_app(dir.path().join("accounts.toml"));
    press(
        &mut app,
        &[
            "A",
            "me@gmail.com",
            "<tab>",
            "<tab>",
            "<tab>",
            "<c-u>",
            "x",
            "<tab>",
            "<tab>",
            "<tab>",
            "pw",
            "<enter>",
        ],
    );
    let screen = app::screen(&mut app, 100, 40);
    assert!(screen.contains("the port is a number"), "{screen}");
    assert!(!screen.contains("apppasswords"), "{screen}");
}

#[test]
fn ctrl_o_opens_the_page_an_app_password_is_made_on() {
    static OPENED: Mutex<Vec<String>> = Mutex::new(Vec::new());
    let dir = tempfile::tempdir().unwrap();
    let mut app = empty_app(dir.path().join("accounts.toml"));
    app.browse = |url| OPENED.lock().unwrap().push(url.to_string());
    press(&mut app, &["A", "me@example.edu", "<c-o>"]);
    assert!(
        OPENED.lock().unwrap().is_empty(),
        "no page for an unknown provider"
    );
    assert!(!app::screen(&mut app, 120, 40).contains("^o"));
    press(&mut app, &["<c-u>", "me@gmail.com"]);
    assert!(app::screen(&mut app, 120, 40).contains("^o"));
    press(&mut app, &["<tab>", "<c-o>"]);
    assert_eq!(
        *OPENED.lock().unwrap(),
        ["https://myaccount.google.com/apppasswords"]
    );
    assert!(app.status.contains("opened"), "{}", app.status);
    assert_eq!(
        app.dialog.form().unwrap().fields[app::NAME],
        "gmail.com",
        "nothing typed"
    );
}

#[test]
fn the_demos_account_is_not_changed() {
    let mut app = demo();
    app.fix_accounts();
    press(&mut app, &["A"]);
    assert!(app.dialog.form().is_none());
    assert!(app.status.contains("without --demo"));
    assert!(!app.rows.contains(&app::Row::Add));
}

#[test]
fn the_accounts_file_keeps_its_comments_as_accounts_come_and_go() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("accounts.toml");
    std::fs::write(
        &path,
        "# my mail\n[[account]]\nname = \"Home\" # the old one\nemail = \"a@b.c\"\nhost = \"imap.b.c\"\n",
    )
    .unwrap();
    let mut work = crate::setup::account(
        "Work",
        "me@example.edu",
        "imap.example.edu",
        993,
        "me@example.edu",
    );
    work.password_command = Some("pass mail".to_string());
    crate::config::add(&path, &work).unwrap();
    assert!(crate::config::add(&path, &work).is_err(), "a name twice");
    let names: Vec<String> = crate::config::load(&path)
        .unwrap()
        .into_iter()
        .map(|a| a.name)
        .collect();
    assert_eq!(names, ["Home", "Work"]);
    let removed = crate::setup::remove(&path, "home", true).unwrap();
    assert_eq!(removed.0.host, "imap.b.c");
    let file = std::fs::read_to_string(&path).unwrap();
    assert!(file.starts_with("# my mail\n"), "{file}");
    assert!(
        !file.contains("Home") && file.contains("name = \"Work\""),
        "{file}"
    );
    assert!(!file.contains("port") && !file.contains("user"), "{file}");
    assert!(crate::setup::remove(&path, "Home", true).is_err());
}

#[test]
fn providers_servers_are_guessed_from_the_address() {
    use crate::config::guess;
    assert_eq!(guess("x@gmail.com").0, "imap.gmail.com");
    assert!(guess("x@gmail.com").1.unwrap().contains("app password"));
    assert_eq!(
        guess("x@example.edu"),
        ("imap.example.edu".to_string(), None)
    );
    assert_eq!(guess("nothing").0, "");
    assert_eq!(crate::config::guess_smtp("x@gmail.com"), "smtp.gmail.com");
    assert_eq!(
        crate::config::guess_smtp("x@outlook.com"),
        "smtp.office365.com"
    );
    assert_eq!(
        crate::config::guess_smtp("x@example.edu"),
        "smtp.example.edu"
    );
}

#[test]
fn an_app_password_page_is_known_for_the_providers_that_take_one() {
    use crate::config::app_password_help;
    let page = |email, host| app_password_help(email, host).map(|(_, page)| page);
    assert_eq!(
        page("x@gmail.com", ""),
        Some("https://myaccount.google.com/apppasswords")
    );
    assert!(page("x@icloud.com", "").is_some());
    assert!(page("x@yahoo.com", "").is_some());
    // An address of its own domain, on Gmail's server, is Gmail's.
    assert_eq!(
        page("x@example.edu", "imap.gmail.com"),
        page("x@gmail.com", "")
    );
    assert_eq!(
        page("x@outlook.com", ""),
        None,
        "it signs in in the browser"
    );
    assert_eq!(page("x@example.edu", "imap.example.edu"), None);
}

#[test]
fn an_app_password_is_taken_with_its_spaces_or_without() {
    use crate::config::app_password;
    let gmail = "imap.gmail.com";
    // Yahoo's and Fastmail's are shown in groups of four too.
    for host in ["imap.mail.yahoo.com", "imap.fastmail.com"] {
        assert_eq!(
            app_password(host, "ab12 cd34 ef56 gh78"),
            "ab12cd34ef56gh78"
        );
    }
    assert_eq!(
        app_password(gmail, "abcdefgh ijklmnop"),
        "abcdefgh ijklmnop",
        "not in groups of four"
    );
    assert_eq!(
        app_password(gmail, "abcd efgh ijkl mnop"),
        "abcdefghijklmnop"
    );
    assert_eq!(
        app_password(gmail, " abcd efgh ijkl mnop "),
        "abcdefghijklmnop"
    );
    assert_eq!(app_password(gmail, "abcdefghijklmnop"), "abcdefghijklmnop");
    // Not an app password's shape: kept as typed.
    assert_eq!(app_password(gmail, "my pass word"), "my pass word");
    // Elsewhere a space may be in a password.
    assert_eq!(
        app_password("imap.example.edu", "abcd efgh ijkl mnop"),
        "abcd efgh ijkl mnop"
    );
}

#[test]
fn with_no_accounts_it_says_how_to_add_one() {
    let mut app = App::new(Vec::new(), PathBuf::from("/nowhere/accounts.toml"));
    let screen = app::screen(&mut app, 120, 40);
    assert!(screen.contains("No accounts yet."), "{screen}");
    assert!(screen.contains("letterbox --demo"), "{screen}");
}

// ---- the mouse ----

fn click_at(app: &mut App, target: app::Target) {
    let _ = app::screen(app, 120, 40);
    let (rect, _) = *app
        .targets
        .iter()
        .rev()
        .find(|(_, t)| *t == target)
        .unwrap_or_else(|| panic!("no {target:?} on the screen"));
    app.mouse(mouse(
        MouseEventKind::Down(MouseButton::Left),
        rect.x,
        rect.y,
    ));
    app.settle(Duration::from_secs(5));
}

use ratatui::crossterm::event::{
    KeyCode, KeyEvent, KeyModifiers, MouseButton, MouseEvent, MouseEventKind,
};

fn mouse(kind: MouseEventKind, column: u16, row: u16) -> MouseEvent {
    MouseEvent {
        kind,
        column,
        row,
        modifiers: KeyModifiers::NONE,
    }
}

#[test]
fn the_forms_fields_and_buttons_are_clicked() {
    let (account, _) = server();
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("accounts.toml");
    let mut app = empty_app(path.clone());
    press(&mut app, &["A", "me@example.com"]);
    click_at(&mut app, app::Target::Field(2));
    assert_eq!(app.dialog.form().unwrap().at, 2);
    press(&mut app, &["<c-u>", "127.0.0.1", "<tab>", "<c-u>"]);
    press(&mut app, &[&account.port.unwrap().to_string()]);
    click_at(&mut app, app::Target::Field(4));
    press(&mut app, &["<c-u>", "me"]);
    click_at(&mut app, app::Target::Field(7));
    click_at(&mut app, app::Target::Field(7));
    assert_eq!(
        app.dialog.form().unwrap().keeper,
        1,
        "a second click picks the vault"
    );
    click_at(&mut app, app::Target::Field(7));
    assert_eq!(
        app.dialog.form().unwrap().keeper,
        0,
        "and a third the keyring again"
    );
    click_at(&mut app, app::Target::Field(6));
    press(&mut app, &["secret"]);
    click_at(&mut app, app::Target::Button(app::Button::Add));
    assert!(
        app.dialog.form().is_none(),
        "{:?}",
        app.dialog.form().and_then(|f| f.note.clone())
    );
    assert_eq!(crate::config::load(&path).unwrap().len(), 1);

    press(&mut app, &["h", "D"]);
    click_at(&mut app, app::Target::Button(app::Button::Keep));
    assert!(!matches!(app.dialog, app::Dialog::Remove(_)));
    assert_eq!(crate::config::load(&path).unwrap().len(), 1);
    press(&mut app, &["D"]);
    click_at(&mut app, app::Target::Button(app::Button::Remove));
    assert!(crate::config::load(&path).unwrap().is_empty());
}

#[test]
fn the_keys_at_the_foot_are_clicked_and_the_help_scrolls() {
    let mut app = demo();
    click_at(
        &mut app,
        app::Target::Key(KeyEvent::new(KeyCode::Char('?'), KeyModifiers::NONE)),
    );
    assert!(matches!(app.dialog, app::Dialog::Help(_)));
    let screen = app::screen(&mut app, 60, 12);
    assert!(screen.contains("j/k or the wheel scroll"), "{screen}");
    let (rect, _) = *app
        .targets
        .iter()
        .find(|(_, t)| *t == app::Target::Popup)
        .unwrap();
    for _ in 0..3 {
        app.mouse(mouse(MouseEventKind::ScrollDown, rect.x + 1, rect.y + 1));
    }
    let _ = app::screen(&mut app, 60, 12);
    assert!(matches!(app.dialog, app::Dialog::Help(3)));
    // The wheel does not run it off its end.
    for _ in 0..100 {
        app.mouse(mouse(MouseEventKind::ScrollDown, rect.x + 1, rect.y + 1));
    }
    let end = app::screen(&mut app, 60, 12);
    assert!(end.contains("quit"), "{end}");
    app.key(ratatui::crossterm::event::KeyEvent::new(
        KeyCode::Esc,
        KeyModifiers::NONE,
    ));
    assert!(!matches!(app.dialog, app::Dialog::Help(_)));
    click_at(
        &mut app,
        app::Target::Key(KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE)),
    );
    assert!(app.message().is_some());
}

// ---- the vault ----

#[test]
fn the_vault_keeps_passwords_encrypted_with_a_passphrase() {
    let (account, _) = server();
    let dir = tempfile::tempdir().unwrap();
    let vault_at = dir.path().join("data/vault.age");
    // SAFETY: tests that read it are this one alone.
    unsafe { std::env::set_var("LETTERBOX_VAULT", &vault_at) };
    crate::vault::lock();
    let path = dir.path().join("accounts.toml");
    microsoft();
    let mut app = App::new(Vec::new(), path.clone());
    app.keepers = || vec![Keeper::Keyring, Keeper::Vault];
    let port = account.port.unwrap().to_string();
    press(
        &mut app,
        &[
            "A",
            "me@example.com",
            "<tab>",
            "<tab>",
            "<c-u>",
            "127.0.0.1",
            "<tab>",
            "<c-u>",
        ],
    );
    press(
        &mut app,
        &[
            &port, "<tab>", "<c-u>", "me", "<tab>", "<tab>", "secret", "<tab>", "<right>",
        ],
    );
    let form = app.dialog.form().unwrap();
    assert_eq!(form.kept(), Some(Keeper::Vault));
    assert_eq!(
        form.shown(),
        [0, 1, 2, 3, 4, 5, 6, 7, 8, 9],
        "a new vault's passphrase, twice"
    );
    press(
        &mut app,
        &[
            "<tab>",
            "correct horse",
            "<tab>",
            "correct house",
            "<enter>",
        ],
    );
    let form = app.dialog.form().expect("still open");
    assert_eq!(form.note.as_deref(), Some("the two passphrases differ"));
    assert!(!vault_at.exists());
    press(&mut app, &["<c-u>", "correct horse", "<enter>"]);
    assert!(
        app.dialog.form().is_none(),
        "{:?}",
        app.dialog.form().and_then(|f| f.note.clone())
    );
    assert_eq!(app.shown.len(), 3);

    let file = std::fs::read_to_string(&path).unwrap();
    assert!(
        file.contains("keep = \"vault\"") && !file.contains("secret"),
        "{file}"
    );
    let sealed = std::fs::read(&vault_at).unwrap();
    assert!(sealed.starts_with(b"age-encryption.org/v1\n"));
    assert!(!String::from_utf8_lossy(&sealed).contains("secret"));
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let mode = std::fs::metadata(&vault_at).unwrap().permissions().mode();
        assert_eq!(mode & 0o777, 0o600);
    }

    // Opened again, as letterbox does as it starts.
    crate::vault::lock();
    let e = crate::vault::unlock("correct house").unwrap_err();
    assert_eq!(e.to_string(), "that is not the vault's passphrase");
    crate::vault::unlock("correct horse").unwrap();
    let saved = crate::config::load(&path).unwrap().remove(0);
    assert_eq!(saved.password().unwrap(), "secret");

    press(&mut app, &["h", "D", "y"]);
    assert!(crate::config::load(&path).unwrap().is_empty());
    assert!(crate::vault::get("127.0.0.1", "me").is_err(), "forgotten");
    crate::vault::lock();
    unsafe { std::env::remove_var("LETTERBOX_VAULT") };
}

/// The system's own store, by keyring-rs, written to, read and emptied:
/// run by hand, as it touches the keychain of whoever runs it.
#[test]
#[ignore]
fn the_systems_store_keeps_a_password() {
    let (host, user) = ("letterbox.test.invalid", "probe@letterbox.test");
    crate::secret::store(Keeper::Keyring, host, user, "p'w \"d").unwrap();
    assert_eq!(
        crate::secret::find(Keeper::Keyring, host, user).unwrap(),
        "p'w \"d"
    );
    let mut a = crate::setup::account("T", user, host, 993, user);
    a.keep = Keeper::Keyring;
    assert!(crate::secret::forget(&a));
    assert!(crate::secret::find(Keeper::Keyring, host, user).is_err());
}

// ---- Microsoft, as letterbox signs in with it ----

/// A stand-in for Microsoft's sign-in, on this machine, which letterbox is
/// pointed at in every test: an address at `uni.example` is its own; a code
/// is taken from the browser if its verifier answers the challenge the
/// browser was given; a device's code is taken on the second time it is
/// asked; tokens are renewed.
fn microsoft() -> &'static str {
    static AT: std::sync::OnceLock<String> = std::sync::OnceLock::new();
    AT.get_or_init(|| {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let at = format!("http://127.0.0.1:{}", listener.local_addr().unwrap().port());
        // SAFETY: set once, before any test signs in.
        unsafe { std::env::set_var("LETTERBOX_MS_LOGIN", &at) };
        std::thread::spawn(move || {
            for tcp in listener.incoming().flatten() {
                std::thread::spawn(move || answer_as_microsoft(tcp));
            }
        });
        at
    })
}

/// The pages the tests' browser was shown, coming back to letterbox.
static PAGES: Mutex<Vec<String>> = Mutex::new(Vec::new());

/// Whether the browser was shown `said`, within a second or two: it reads
/// the page on a thread of its own.
fn shown(said: &str) -> bool {
    let until = std::time::Instant::now() + Duration::from_secs(2);
    loop {
        if PAGES.lock().unwrap().iter().any(|p| p.contains(said)) {
            return true;
        }
        if std::time::Instant::now() > until {
            return false;
        }
        std::thread::sleep(Duration::from_millis(20));
    }
}

/// The challenge the test's browser was given, to check the verifier by.
static CHALLENGE: Mutex<Option<String>> = Mutex::new(None);
static DEVICES_ASKED: std::sync::atomic::AtomicUsize = std::sync::atomic::AtomicUsize::new(0);

fn answer_as_microsoft(tcp: TcpStream) {
    use std::io::Read;
    let mut reader = BufReader::new(tcp.try_clone().unwrap());
    let mut first = String::new();
    reader.read_line(&mut first).unwrap();
    let mut length = 0;
    loop {
        let mut header = String::new();
        reader.read_line(&mut header).unwrap();
        if header.trim().is_empty() {
            break;
        }
        if let Some(v) = header.to_lowercase().strip_prefix("content-length:") {
            length = v.trim().parse().unwrap();
        }
    }
    let mut body = vec![0; length];
    reader.read_exact(&mut body).unwrap();
    let form: std::collections::HashMap<String, String> = String::from_utf8_lossy(&body)
        .split('&')
        .filter_map(|p| p.split_once('='))
        .map(|(k, v)| (k.to_string(), v.replace("%3A", ":").replace("%2F", "/")))
        .collect();
    let target = first.split_whitespace().nth(1).unwrap_or_default();
    let tokens = r#"{"access_token":"access-1","refresh_token":"refresh-1","expires_in":3600}"#;
    let json = if target.starts_with("/getuserrealm.srf") {
        let ours = target.contains("uni.example");
        format!(
            r#"{{"NameSpaceType":"{}"}}"#,
            if ours { "Managed" } else { "Unknown" }
        )
    } else if target.ends_with("/devicecode") {
        let n = DEVICES_ASKED.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
        format!(
            r#"{{"device_code":"device-{n}","user_code":"LBX-CODE","verification_uri":"https://microsoft.example/device","interval":1,"expires_in":900}}"#
        )
    } else if target.ends_with("/token") {
        match form.get("grant_type").map(String::as_str) {
            Some("authorization_code") => {
                use base64::Engine;
                use sha2::Digest;
                let verifier = form.get("code_verifier").cloned().unwrap_or_default();
                let answered = base64::engine::general_purpose::URL_SAFE_NO_PAD
                    .encode(sha2::Sha256::digest(verifier.as_bytes()));
                let challenge = CHALLENGE.lock().unwrap().clone();
                if form.get("code").map(String::as_str) == Some("code-1")
                    && challenge.as_deref() == Some(answered.as_str())
                {
                    tokens.to_string()
                } else {
                    r#"{"error":"invalid_grant","error_description":"AADSTS70000: bad code"}"#
                        .to_string()
                }
            }
            Some(grant) if grant.contains("device_code") => {
                // Asked first while no one has signed in, then signed in.
                static POLLED: Mutex<Vec<String>> = Mutex::new(Vec::new());
                let code = form.get("device_code").cloned().unwrap_or_default();
                let mut polled = POLLED.lock().unwrap();
                if polled.contains(&code) {
                    tokens.to_string()
                } else {
                    polled.push(code);
                    r#"{"error":"authorization_pending"}"#.to_string()
                }
            }
            Some("refresh_token") => r#"{"access_token":"access-2","expires_in":3600}"#.to_string(),
            _ => r#"{"error":"unsupported_grant_type"}"#.to_string(),
        }
    } else {
        r#"{"error":"not_found"}"#.to_string()
    };
    let mut out = tcp;
    let _ = write!(
        out,
        "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{json}",
        json.len()
    );
}

/// The query of `url`, by name.
fn query(url: &str) -> std::collections::HashMap<String, String> {
    url.split_once('?')
        .map(|(_, q)| q)
        .unwrap_or_default()
        .split('&')
        .filter_map(|p| p.split_once('='))
        .map(|(k, v)| (k.to_string(), v.replace("%3A", ":").replace("%2F", "/")))
        .collect()
}

/// A browser in which you sign in on Microsoft's page: it comes back to
/// letterbox with `back` added to what letterbox asked to have back.
fn come_back(url: &str, back: &str) -> String {
    let q = query(url);
    *CHALLENGE.lock().unwrap() = q.get("code_challenge").cloned();
    let redirect = q.get("redirect_uri").cloned().unwrap();
    let state = q.get("state").cloned().unwrap();
    let to = redirect.trim_start_matches("http://localhost:").to_string();
    let request = format!("GET /?{back}&state={state} HTTP/1.1\r\nHost: localhost\r\n\r\n");
    std::thread::spawn(move || {
        let mut tcp = TcpStream::connect(("127.0.0.1", to.parse::<u16>().unwrap())).unwrap();
        tcp.write_all(request.as_bytes()).unwrap();
        let mut page = String::new();
        use std::io::Read;
        let _ = tcp.read_to_string(&mut page);
        page
    })
    .join()
    .unwrap()
}

#[test]
fn an_outlook_account_is_signed_into_in_the_browser() {
    let (account, _) = server();
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("accounts.toml");
    let mut app = empty_app(path.clone());
    app.browse = |url| {
        let page = come_back(url, "code=code-1");
        PAGES.lock().unwrap().push(page);
    };
    press(&mut app, &["A", "me@uni.example", "<tab>"]);
    let form = app.dialog.form().unwrap();
    assert_eq!(
        form.auth,
        crate::config::Auth::Microsoft,
        "Microsoft says so"
    );
    assert_eq!(form.fields[2], "outlook.office365.com");
    assert!(
        !form.shown().contains(&app::PASSWORD),
        "no password asked for"
    );
    let port = account.port.unwrap().to_string();
    press(
        &mut app,
        &["<tab>", "<c-u>", "127.0.0.1", "<tab>", "<c-u>", &port],
    );
    press(&mut app, &["<tab>", "<c-u>", "me", "<tab>", "<enter>"]);
    assert!(
        app.dialog.form().is_none(),
        "{:?}",
        app.dialog.form().and_then(|f| f.note.clone())
    );
    assert_eq!(app.status, "added uni.example");
    assert_eq!(app.shown.len(), 3, "signed in to IMAP with the token");
    let file = std::fs::read_to_string(&path).unwrap();
    assert!(file.contains("auth = \"microsoft\""), "{file}");
    assert!(
        !file.contains("refresh") && !file.contains("access"),
        "{file}"
    );
    assert!(shown("Signed in. You may close this tab"));
}

#[test]
fn an_outlook_account_is_signed_into_with_a_code_on_another_device() {
    microsoft();
    let stop = std::sync::atomic::AtomicBool::new(false);
    let mut shown = None;
    let tokens = crate::oauth::sign_in(
        "me@uni.example",
        "common",
        crate::oauth::CLIENT_ID,
        &stop,
        |_| {},
        |w| shown = w.device.clone(),
    )
    .unwrap();
    assert_eq!(
        shown,
        Some((
            "https://microsoft.example/device".to_string(),
            "LBX-CODE".to_string()
        ))
    );
    assert_eq!(
        (tokens.access.as_str(), tokens.refresh.as_str()),
        ("access-1", "refresh-1")
    );
}

#[test]
fn a_sign_in_refused_in_the_browser_is_said() {
    microsoft();
    let stop = std::sync::atomic::AtomicBool::new(false);
    let e = crate::oauth::sign_in(
        "me@uni.example",
        "common",
        crate::oauth::CLIENT_ID,
        &stop,
        |url| {
            let page = come_back(
                url,
                "error=access_denied&error_description=Your+admin+said+no",
            );
            PAGES.lock().unwrap().push(page);
        },
        |_| {},
    )
    .unwrap_err();
    assert_eq!(e.to_string(), "Your admin said no");
    assert!(shown("<p>Your admin said no"), "the browser is told too");
}

#[test]
fn a_token_running_out_is_renewed() {
    microsoft();
    let mut a = crate::setup::account("O", "me@uni.example", "outlook.example", 993, "me");
    a.auth = crate::config::Auth::Microsoft;
    a.password_command = Some("echo unused".to_string());
    crate::oauth::remember(
        &a,
        crate::oauth::Tokens {
            access: "access-1".to_string(),
            refresh: "refresh-1".to_string(),
            until: std::time::Instant::now(),
        },
    );
    assert_eq!(crate::oauth::access(&a).unwrap(), "access-2");
    assert_eq!(
        crate::oauth::access(&a).unwrap(),
        "access-2",
        "kept, not renewed again"
    );
}

// ---- for AI agents: MCP ----

fn call(server: &mut crate::mcp::Server, tool: &str, args: serde_json::Value) -> serde_json::Value {
    let reply = server
        .answer(&serde_json::json!({
            "jsonrpc": "2.0", "id": 7, "method": "tools/call",
            "params": { "name": tool, "arguments": args }
        }))
        .unwrap();
    reply["result"].clone()
}

#[test]
fn an_agent_lists_searches_reads_and_marks_mail() {
    use serde_json::json;
    let mut server = crate::mcp::Server::new(&PathBuf::from("/nowhere"), true).unwrap();
    let init = server
        .answer(&json!({ "jsonrpc": "2.0", "id": 1, "method": "initialize",
            "params": { "protocolVersion": "2025-06-18" } }))
        .unwrap();
    assert_eq!(init["result"]["serverInfo"]["name"], "letterbox");
    assert!(
        init["result"]["instructions"]
            .as_str()
            .unwrap()
            .contains("never followed")
    );
    assert!(
        server
            .answer(&json!({ "jsonrpc": "2.0", "method": "notifications/initialized" }))
            .is_none()
    );
    let tools = server
        .answer(&json!({ "jsonrpc": "2.0", "id": 2, "method": "tools/list" }))
        .unwrap();
    let names: Vec<&str> = tools["result"]["tools"]
        .as_array()
        .unwrap()
        .iter()
        .map(|t| t["name"].as_str().unwrap())
        .collect();
    assert_eq!(
        names,
        [
            "list_accounts",
            "list_messages",
            "search_messages",
            "read_message",
            "mark_message"
        ]
    );

    let accounts = call(&mut server, "list_accounts", json!({}));
    let inbox = &accounts["structuredContent"]["accounts"][0]["folders"][0];
    assert_eq!(
        (inbox["path"].as_str(), inbox["total"].as_u64()),
        (Some("INBOX"), Some(INBOX as u64))
    );

    let listed = call(&mut server, "list_messages", json!({ "limit": 2 }));
    let listed = &listed["structuredContent"];
    assert_eq!(listed["shown"], 2);
    assert_eq!(listed["more"], true);
    assert_eq!(listed["messages"][0]["from"], "Ada Lovelace");
    let uid = listed["messages"][0]["uid"].as_u64().unwrap();

    let found = call(&mut server, "search_messages", json!({ "query": "helix" }));
    assert_eq!(
        found["structuredContent"]["messages"][0]["subject"],
        "Photo 51"
    );

    let read = call(
        &mut server,
        "read_message",
        json!({ "uid": uid, "max_chars": 50 }),
    );
    let read = &read["structuredContent"];
    assert_eq!(read["text"].as_str().unwrap().chars().count(), 50);
    assert_eq!(read["more"], "from_char 50 for more");
    let rest = call(
        &mut server,
        "read_message",
        json!({ "uid": uid, "from_char": 50 }),
    );
    assert!(
        rest["structuredContent"]["text"]
            .as_str()
            .unwrap()
            .contains("sets out how")
    );
    let again = call(&mut server, "list_messages", json!({ "limit": 1 }));
    assert_eq!(
        again["structuredContent"]["messages"][0]["seen"], false,
        "read, not marked"
    );

    let marked = call(
        &mut server,
        "mark_message",
        json!({ "uid": uid, "seen": true, "flagged": true }),
    );
    assert_eq!(marked["isError"], false);
    let after = call(&mut server, "list_messages", json!({ "limit": 1 }));
    let first = &after["structuredContent"]["messages"][0];
    assert_eq!(
        (first["seen"].as_bool(), first["flagged"].as_bool()),
        (Some(true), Some(true))
    );

    let wrong = call(&mut server, "list_messages", json!({ "account": "Nope" }));
    assert_eq!(wrong["isError"], true);
    assert!(
        wrong["content"][0]["text"]
            .as_str()
            .unwrap()
            .contains("there are Demo")
    );
    let none = call(&mut server, "mark_message", json!({ "uid": uid }));
    assert_eq!(none["content"][0]["text"], "say seen, flagged, or both");
}

#[test]
fn an_agent_is_told_a_vaults_account_is_not_for_it() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("accounts.toml");
    std::fs::write(
        &path,
        "[[account]]\nname = \"Safe\"\nemail = \"a@b.c\"\nhost = \"imap.b.c\"\nkeep = \"vault\"\n",
    )
    .unwrap();
    crate::vault::lock();
    let mut server = crate::mcp::Server::new(&path, false).unwrap();
    let r = call(&mut server, "list_messages", serde_json::json!({}));
    assert_eq!(r["isError"], true);
    assert!(
        r["content"][0]["text"]
            .as_str()
            .unwrap()
            .contains("keep it in the keyring")
    );
}

#[test]
fn deleting_moves_to_trash_and_back() {
    let mut app = demo();
    assert_eq!(app.shown[0].from, "Ada Lovelace");
    press(&mut app, &["d"]);
    assert!(
        app.shown.iter().all(|s| s.from != "Ada Lovelace"),
        "{}",
        app.status
    );
    assert!(app.status.contains("7 days"), "{}", app.status);
    press(&mut app, &["h", "j", "j", "j", "j", "j"]);
    assert_eq!(app.folder_name(), "Trash", "{}", app.status);
    assert_eq!(app.shown[0].from, "Ada Lovelace");
    press(&mut app, &["d"]);
    assert!(app.status.contains("Inbox"), "{}", app.status);
    assert!(app.shown.iter().all(|s| s.from != "Ada Lovelace"));
    press(&mut app, &["h", "k", "k", "k", "k", "k"]);
    assert_eq!(app.folder_name(), "Inbox");
    assert_eq!(app.shown[0].from, "Ada Lovelace");
}

#[test]
fn a_folder_is_chosen_as_trash_when_the_server_names_none() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("accounts.toml");
    std::fs::write(
        &path,
        "[[account]]\nname = \"Demo\"\nemail = \"you@example.com\"\nhost = \"imap.example.com\"\n",
    )
    .unwrap();
    let mut app = demo();
    app.config = path.clone();
    // A server that marks nothing as Trash: the folder of that name is
    // only a name, until one is chosen.
    for folder in &mut app.accounts[0].folders {
        if folder.role == Role::Trash {
            folder.role = Role::Other;
        }
    }
    press(&mut app, &["d", "<esc>"]);
    assert!(!matches!(app.dialog, app::Dialog::Trash(_)));
    assert!(app.status.contains("not moved"), "{}", app.status);
    assert!(app.shown.iter().any(|s| s.from == "Ada Lovelace"));

    press(&mut app, &["d"]);
    let screen = app::screen(&mut app, 100, 40);
    assert!(
        screen.contains("Which of these folders") && screen.contains("Archive"),
        "{screen}"
    );
    let at = app
        .dialog
        .trash_mut()
        .unwrap()
        .folders
        .iter()
        .position(|(_, name)| name == "Archive")
        .unwrap();
    app.dialog.trash_mut().unwrap().at = at;
    // ⏎ at the foot, clicked, chooses it as ⏎ does.
    click_at(
        &mut app,
        app::Target::Key(KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE)),
    );
    assert!(
        !matches!(app.dialog, app::Dialog::Trash(_)),
        "{}",
        app.status
    );
    assert!(app.status.contains("moved to Archive"), "{}", app.status);
    assert!(app.shown.iter().all(|s| s.from != "Ada Lovelace"));
    let file = std::fs::read_to_string(&path).unwrap();
    assert!(file.contains("trash = \"Archive\""), "{file}");
    assert!(file.contains("host = \"imap.example.com\""), "{file}");

    press(&mut app, &["h", "j", "j", "j"]);
    assert_eq!(app.folder_name(), "Archive", "{}", app.status);
    assert!(app.shown.iter().any(|s| s.from == "Ada Lovelace"));
    press(&mut app, &["d"]);
    assert!(app.status.contains("put back in Inbox"), "{}", app.status);
}

#[test]
fn a_trash_record_that_cannot_be_read_is_left_as_it_is() {
    let path = std::env::temp_dir().join(format!(
        "letterbox-trash-broken-{}-{}.toml",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos()
    ));
    let text = "this is not a ledger\n";
    std::fs::write(&path, text).unwrap();
    let mut ledger = crate::trash::Ledger::open(path.clone());
    assert!(ledger.fault().is_some());
    assert!(ledger.due("a@b.c", i64::MAX).is_empty());
    let err = ledger
        .record(crate::trash::Entry {
            account: "a@b.c".into(),
            message_id: "<x@x>".into(),
            folder: "Trash".into(),
            uid: 1,
            origin: "INBOX".into(),
            at: 0,
        })
        .unwrap_err();
    assert!(err.to_string().contains("cannot read"), "{err}");
    assert_eq!(std::fs::read_to_string(&path).unwrap(), text);
    let _ = std::fs::remove_file(&path);
}

#[test]
fn a_message_trashed_for_eight_days_is_removed() {
    let mut ledger = scratch_ledger();
    let now = jiff::Timestamp::now().as_second();
    let entry = |uid, days| crate::trash::Entry {
        account: "a@b.c".into(),
        message_id: format!("<{uid}@x>"),
        folder: "Trash".into(),
        uid,
        origin: "INBOX".into(),
        at: now - i64::from(days) * 24 * 60 * 60,
    };
    ledger.record(entry(3, 6)).unwrap();
    ledger.record(entry(4, 8)).unwrap();
    let due: Vec<u32> = ledger
        .due("a@b.c", now)
        .into_iter()
        .map(|e| e.uid)
        .collect();
    assert_eq!(due, [4]);

    let mut app = demo();
    press(&mut app, &["d"]);
    app.ledger
        .backdate(jiff::Timestamp::now().as_second() - 8 * 24 * 60 * 60);
    press(&mut app, &["h", "j", "j", "j", "j", "j"]);
    assert_eq!(app.folder_name(), "Trash");
    assert!(
        app.shown.iter().all(|s| s.from != "Ada Lovelace"),
        "{:?}",
        app.shown
            .iter()
            .map(|s| s.from.as_str())
            .collect::<Vec<_>>()
    );
}

#[test]
fn emacs_keys_kill_and_yank() {
    use crate::edit::{self, Buffer, Done};
    use ratatui::crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
    let mut buffer = Buffer::from_text("hello");
    let mut prefix = false;
    let ctrl = |c| KeyEvent::new(KeyCode::Char(c), KeyModifiers::CONTROL);
    assert_eq!(
        edit::handle(&mut buffer, &mut prefix, ctrl('e')),
        Done::Stay
    );
    assert_eq!(buffer.col(), 5);
    edit::handle(&mut buffer, &mut prefix, ctrl('a'));
    assert_eq!(buffer.col(), 0);
    edit::handle(&mut buffer, &mut prefix, ctrl('k'));
    assert_eq!(buffer.text(), "\n");
    edit::handle(&mut buffer, &mut prefix, ctrl('y'));
    assert_eq!(buffer.text(), "hello\n");
}

#[test]
fn a_reply_is_addressed_to_the_sender() {
    let raw = b"From: Ada Lovelace <ada@analytical.example>\r\n\
        To: You <you@example.com>\r\nSubject: Notes on the engine\r\n\
        Message-ID: <1@demo>\r\nReferences: <0@demo>\r\n\
        \r\nHello\r\n"
        .to_vec();
    let message = mail::message(1, raw).unwrap();
    let (text, row) = crate::compose::reply(&message);
    assert!(
        text.starts_with("To: Ada Lovelace <ada@analytical.example>\n"),
        "{text}"
    );
    assert!(
        text.contains("\nSubject: Re: Notes on the engine\n"),
        "{text}"
    );
    assert!(text.contains("> Hello\n"), "{text}");
    assert_eq!(row, 4);
    assert_eq!(crate::compose::reply_subject("éé"), "Re: éé");
    assert_eq!(crate::compose::reply_subject("😀 notes"), "Re: 😀 notes");
    assert_eq!(crate::compose::reply_subject("Re: éé"), "Re: éé");
    let subject = format!("{}é{}", "a".repeat(44), "b".repeat(10));
    let draft = crate::compose::Draft {
        to: vec!["a@b.c".into()],
        cc: Vec::new(),
        subject: subject.clone(),
        attach: Vec::new(),
        body: "hi".into(),
    };
    let raw = crate::compose::bytes("me@example.com", &draft, None, None, &[]);
    assert_eq!(mail::message(1, raw).unwrap().subject, subject);
    let (id, refs) = crate::compose::thread(&message);
    assert_eq!(id.as_deref(), Some("<1@demo>"));
    assert!(refs.unwrap().contains("<0@demo>"));
}

#[test]
fn replying_opens_the_message_and_the_demo_keeps_what_is_sent() {
    let mut app = demo();
    press(&mut app, &["<c-r>"]);
    let text = app.compose.as_ref().unwrap().buffer.text();
    assert!(
        text.contains("To: Ada Lovelace <ada@analytical.example>"),
        "{text}"
    );
    assert!(text.contains("Subject: Re: Notes on the engine"), "{text}");
    press(&mut app, &["<c-c>", "<c-k>"]);
    assert!(app.compose.is_none(), "unchanged, it goes at once");
    assert!(app.status.contains("not sent"), "{}", app.status);

    press(
        &mut app,
        &[
            "c",
            "ada@analytical.example",
            "<c-n>",
            "<c-n>",
            "Hello",
            "<c-n>",
            "<c-n>",
            "Hi",
            "<c-c>",
            "<c-c>",
        ],
    );
    assert!(app.compose.is_none(), "{}", app.status);
    assert!(app.status.contains("not delivered"), "{}", app.status);
    press(&mut app, &["h", "j", "j"]);
    assert_eq!(app.folder_name(), "Sent");
    assert!(
        app.shown.iter().any(|s| s.subject == "Hello"),
        "{:?}",
        app.shown
            .iter()
            .map(|s| s.subject.as_str())
            .collect::<Vec<_>>()
    );
}

#[test]
fn imap_moves_appends_and_expunges() {
    let (account, _) = server();
    let mut imap = Imap::new(account);
    let page = imap.page("INBOX", 0, 10).unwrap();
    let summary = page
        .summaries
        .iter()
        .find(|s| s.from.contains("Rosalind"))
        .unwrap();
    let uid = summary.uid;
    let new_uid = imap
        .relocate("INBOX", uid, "Deleted Items", &summary.message_id)
        .unwrap();
    assert!(
        imap.page("INBOX", 0, 10)
            .unwrap()
            .summaries
            .iter()
            .all(|s| s.uid != uid)
    );
    let trash = imap.page("Deleted Items", 0, 10).unwrap();
    assert!(trash.summaries.iter().any(|s| s.uid == new_uid));
    assert_eq!(
        imap.expunge("Deleted Items", new_uid).unwrap(),
        Removal::Gone
    );
    assert_eq!(
        imap.expunge("Deleted Items", new_uid).unwrap(),
        Removal::Missing
    );
    assert!(
        imap.page("Deleted Items", 0, 10)
            .unwrap()
            .summaries
            .is_empty()
    );
    imap.append(
        "INBOX",
        b"From: me@example.com\r\nSubject: no id\r\n\r\nhi\r\n",
        true,
    )
    .unwrap();
    let bare = imap
        .page("INBOX", 0, 20)
        .unwrap()
        .summaries
        .into_iter()
        .find(|s| s.subject == "no id")
        .unwrap();
    let err = imap
        .relocate("INBOX", bare.uid, "Deleted Items", "")
        .unwrap_err();
    assert!(err.to_string().contains("no Message-ID"), "{err}");
    assert!(
        imap.page("INBOX", 0, 20)
            .unwrap()
            .summaries
            .iter()
            .any(|s| s.uid == bare.uid)
    );
    imap.append("Sent Items", b"From: me@example.com\r\n\r\nhi\r\n", true)
        .unwrap();
    assert_eq!(imap.page("Sent Items", 0, 10).unwrap().total, 1);
    imap.create("Later").unwrap();
    assert!(imap.folders().unwrap().iter().any(|f| f.path == "Later"));
}

struct SmtpLog {
    auth: String,
    data: Vec<u8>,
}

fn smtp_server() -> (u16, Arc<Mutex<SmtpLog>>) {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let port = listener.local_addr().unwrap().port();
    let log = Arc::new(Mutex::new(SmtpLog {
        auth: String::new(),
        data: Vec::new(),
    }));
    let shared = log.clone();
    std::thread::spawn(move || {
        let (tcp, _) = listener.accept().unwrap();
        let mut tcp = tcp.try_clone().unwrap();
        let mut lines = BufReader::new(tcp.try_clone().unwrap());
        tcp.write_all(b"220 ready\r\n").unwrap();
        let mut line = String::new();
        loop {
            line.clear();
            if lines.read_line(&mut line).unwrap_or(0) == 0 {
                return;
            }
            let command = line.trim();
            let upper = command.to_ascii_uppercase();
            if upper.starts_with("EHLO") {
                tcp.write_all(b"250-ready\r\n250-8BITMIME\r\n250 AUTH PLAIN\r\n")
                    .unwrap();
            } else if upper.starts_with("AUTH PLAIN") {
                shared.lock().unwrap().auth = command.to_string();
                tcp.write_all(b"235 ok\r\n").unwrap();
            } else if upper.starts_with("MAIL FROM") || upper.starts_with("RCPT TO") {
                tcp.write_all(b"250 ok\r\n").unwrap();
            } else if upper == "DATA" {
                tcp.write_all(b"354 go\r\n").unwrap();
                let mut data = Vec::new();
                let mut data_line = String::new();
                loop {
                    data_line.clear();
                    if lines.read_line(&mut data_line).unwrap_or(0) == 0 {
                        return;
                    }
                    if data_line.trim_end() == "." {
                        break;
                    }
                    let mut stored = data_line.as_str();
                    if let Some(rest) = stored.strip_prefix("..") {
                        data.extend(b".");
                        stored = rest;
                    }
                    data.extend(stored.as_bytes());
                }
                shared.lock().unwrap().data = data;
                tcp.write_all(b"250 ok\r\n").unwrap();
            } else if upper.starts_with("QUIT") {
                tcp.write_all(b"221 bye\r\n").unwrap();
                return;
            } else {
                tcp.write_all(b"500 no\r\n").unwrap();
            }
        }
    });
    (port, log)
}

#[test]
fn smtp_sends_what_was_written_and_keeps_it_in_sent() {
    let (port, log) = smtp_server();
    let (mut account, boxes) = server();
    account.smtp_host = Some("127.0.0.1".into());
    account.smtp_port = Some(port);
    let mut imap = Imap::new(account);
    let raw = b"From: me@example.com\r\nTo: ada@example.com\r\nSubject: hi\r\n\
        Date: Tue, 20 Oct 2026 10:00:00 +0000\r\nMessage-ID: <1@example>\r\n\
        MIME-Version: 1.0\r\nContent-Type: text/plain; charset=utf-8\r\n\
        Content-Transfer-Encoding: 8bit\r\n\r\nHello\r\n.dot\r\n";
    assert_eq!(imap.send(raw).unwrap(), "sent");
    let got = log.lock().unwrap();
    assert!(got.auth.contains("PLAIN"), "{}", got.auth);
    let text = String::from_utf8_lossy(&got.data);
    assert!(text.contains(".dot"), "{text}");
    assert!(!text.contains("..dot"), "{text}");
    drop(got);
    let boxes = boxes.lock().unwrap();
    let sent = boxes.iter().find(|b| b.name == "Sent Items").unwrap();
    assert_eq!(sent.letters.len(), 1);
    assert!(String::from_utf8_lossy(&sent.letters[0].raw).contains(".dot"));
}

#[test]
fn mail_is_not_sent_in_the_clear_off_this_machine() {
    let (mut account, _) = server();
    account.smtp_host = Some("smtp.example.com".into());
    account.security = Security::None;
    let err = crate::smtp::submit(&account, b"From: me@example.com\r\n\r\n\r\n").unwrap_err();
    assert!(err.to_string().contains("unencrypted"), "{err}");
}

#[test]
fn a_body_that_is_not_ascii_is_quoted_when_the_server_cannot_take_it() {
    let raw = b"Content-Transfer-Encoding: 8bit\r\n\r\nCaf\xc3\xa9\r\n";
    let quoted = crate::smtp::for_transfer(raw, false);
    let text = String::from_utf8_lossy(&quoted);
    assert!(text.contains("quoted-printable"), "{text}");
    assert!(text.contains("=C3=A9"), "{text}");
    assert_eq!(crate::smtp::for_transfer(raw, true), raw);
}

// ---- every account's Inbox, Sent and Drafts together ----

/// The demo twice, as the accounts Home and Work; Work's inbox has a
/// message newer than any of Home's.
fn two_accounts() -> App {
    let mut work = Demo::new().called("work");
    work.append(
        "INBOX",
        b"From: Grace Hopper <grace@example.com>\r\nTo: you@example.com\r\n\
          Subject: Newest of all\r\nDate: Sat, 1 Jan 2050 00:00:00 +0000\r\n\
          Message-ID: <newest@example.com>\r\n\r\nHello\r\n",
        false,
    )
    .unwrap();
    let mut app = App::new(
        vec![
            (
                "Home".to_string(),
                Box::new(Demo::new().called("home")) as Box<dyn Store>,
            ),
            ("Work".to_string(), Box::new(work) as Box<dyn Store>),
        ],
        PathBuf::from("/nowhere/accounts.toml"),
    );
    app.ledger = scratch_ledger();
    app.settle(Duration::from_secs(5));
    app
}

/// List the sidebar's row `row`, as if it were picked there, and pick the
/// top of the list: what was picked stays picked as the other accounts'
/// messages come.
fn list_row(app: &mut App, row: app::Row) {
    press(app, &["h"]);
    app.row = app
        .rows
        .iter()
        .position(|r| *r == row)
        .expect("in the sidebar");
    press(app, &["<enter>", "g"]);
}

#[test]
fn every_accounts_inbox_sent_and_drafts_are_in_the_sidebar_together() {
    use app::Row;
    let app = two_accounts();
    assert_eq!(
        app.rows[..5],
        [
            Row::Everyone,
            Row::All(Role::Inbox),
            Row::All(Role::Sent),
            Row::All(Role::Drafts),
            Row::Account(0)
        ]
    );
    // With one account, they would be its own again.
    assert!(!demo().rows.contains(&Row::Everyone));
}

#[test]
fn the_inboxes_together_are_listed_newest_first_each_with_its_account() {
    let mut app = two_accounts();
    // Unread, they are counted together.
    assert_eq!(app.unseen_of(Role::Inbox), Some(UNREAD + UNREAD + 1));
    list_row(&mut app, app::Row::All(Role::Inbox));
    assert_eq!(app.shown.len(), 2 * INBOX + 1, "{}", app.status);
    assert_eq!(app.shown[0].subject, "Newest of all");
    assert_eq!(app.shown[0].account, 1);
    assert_eq!(app.shown.iter().filter(|m| m.account == 0).count(), INBOX);
    let dates: Vec<i64> = app.shown.iter().filter_map(|m| m.date).collect();
    assert!(dates.windows(2).all(|w| w[0] >= w[1]), "{dates:?}");
    let screen = app::screen(&mut app, 140, 40);
    assert!(
        screen.contains("Work") && screen.contains("Home"),
        "{screen}"
    );
}

#[test]
fn a_message_together_with_others_is_read_and_marked_on_its_own_account() {
    let mut app = two_accounts();
    list_row(&mut app, app::Row::All(Role::Inbox));
    press(&mut app, &["<enter>"]);
    let reading = app.reading.as_ref().unwrap();
    assert_eq!((reading.account, reading.folder.as_str()), (1, "INBOX"));
    assert_eq!(app.message().unwrap().subject, "Newest of all");
    let unseen = |app: &App, a: usize| app.accounts[a].folders[0].unseen;
    assert_eq!(unseen(&app, 1), Some(UNREAD), "Work's newest is read");
    assert_eq!(unseen(&app, 0), Some(UNREAD), "Home's are not");
    assert!(app.shown[0].flags.seen);
}

#[test]
fn a_reply_from_the_inboxes_together_is_sent_from_its_account() {
    let mut app = two_accounts();
    list_row(&mut app, app::Row::All(Role::Inbox));
    press(&mut app, &["<c-r>"]);
    assert_eq!(app.compose.as_ref().unwrap().account, 1);
    press(&mut app, &["<esc>"]);
}

#[test]
fn a_message_together_with_others_is_trashed_on_its_own_account() {
    let mut app = two_accounts();
    list_row(&mut app, app::Row::All(Role::Inbox));
    press(&mut app, &["d"]);
    assert!(
        app.shown.iter().all(|m| m.subject != "Newest of all"),
        "{}",
        app.status
    );
    assert_eq!(app.shown.len(), 2 * INBOX);
    let trash = |app: &App, a: usize| {
        app.accounts[a]
            .folders
            .iter()
            .find(|f| f.role == Role::Trash)
            .and_then(|f| f.total)
    };
    assert_eq!(trash(&app, 1), Some(1), "{}", app.status);
    assert_eq!(trash(&app, 0), Some(0));
}

#[test]
fn the_inboxes_together_are_searched_on_every_server() {
    let mut app = two_accounts();
    list_row(&mut app, app::Row::All(Role::Inbox));
    press(&mut app, &["/", "bernoulli numbers step", "<enter>"]);
    let mut accounts: Vec<usize> = app.shown.iter().map(|m| m.account).collect();
    accounts.sort();
    assert_eq!(accounts, [0, 1], "{}", app.status);
}

#[test]
fn folders_together_show_no_message_older_than_one_still_to_come() {
    use app::{Part, merged};
    let summary = |uid, date| mail::Summary {
        uid,
        date: Some(date),
        ..Default::default()
    };
    let part = |account, total, dates: &[i64]| Part {
        account,
        folder: "INBOX".to_string(),
        messages: dates.iter().map(|&d| summary(d as u32, d)).collect(),
        total,
        loading: false,
        error: None,
    };
    // A has more than it listed, older than 90: B's 10 waits for them.
    let parts = [part(0, 200, &[100, 90]), part(1, 2, &[95, 10])];
    let dates: Vec<i64> = merged(&parts).iter().filter_map(|(_, m)| m.date).collect();
    assert_eq!(dates, [100, 95, 90]);
    // A message without a date is not held back: it comes last.
    let mut undated = part(1, 2, &[95]);
    undated.messages.push(mail::Summary {
        uid: 7,
        ..Default::default()
    });
    let parts = [part(0, 200, &[100, 90]), undated];
    let uids: Vec<u32> = merged(&parts).iter().map(|(_, m)| m.uid).collect();
    assert_eq!(uids, [100, 95, 90, 7]);
    // A folder alone is listed as its server numbers it, all of it.
    let alone = [part(0, 200, &[90, 100])];
    let dates: Vec<i64> = merged(&alone).iter().filter_map(|(_, m)| m.date).collect();
    assert_eq!(dates, [90, 100]);
}

#[test]
fn the_inboxes_together_are_listed_by_a_click_too() {
    let mut app = two_accounts();
    let row = app
        .rows
        .iter()
        .position(|r| *r == app::Row::All(Role::Inbox))
        .unwrap();
    click_at(&mut app, app::Target::Row(row));
    assert_eq!(app.folder_name(), "Inbox, all accounts");
    assert_eq!(app.focus, Focus::List);
}

#[test]
fn the_account_removed_from_folders_together_is_the_one_of_the_message_picked() {
    let mut app = two_accounts();
    list_row(&mut app, app::Row::All(Role::Inbox));
    press(&mut app, &["D"]);
    assert!(matches!(app.dialog, app::Dialog::Remove(1)));
    press(&mut app, &["n"]);
}

// ---- replying to all, forwarding, and renaming an account ----

/// A message to you and others: from Ada, to you and Bob, Carol copied.
fn to_several() -> mail::Message {
    let raw = b"From: Ada Lovelace <ada@analytical.example>\r\n\
        To: You <you@example.com>, Bob <bob@example.com>\r\n\
        Cc: Carol <carol@example.com>, ada@analytical.example\r\n\
        Subject: Notes on the engine\r\nMessage-ID: <1@demo>\r\n\
        Date: Mon, 2 Jan 2023 10:00:00 +0000\r\n\r\nHello\r\n"
        .to_vec();
    mail::message(1, raw).unwrap()
}

#[test]
fn a_reply_to_all_is_to_the_sender_with_the_others_copied_but_you() {
    let (text, row) = crate::compose::reply_all(&to_several(), "YOU@example.com");
    assert!(
        text.starts_with("To: Ada Lovelace <ada@analytical.example>\n"),
        "{text}"
    );
    assert!(
        text.contains("\nCc: Bob <bob@example.com>, Carol <carol@example.com>\n"),
        "{text}"
    );
    assert!(
        text.contains("\nSubject: Re: Notes on the engine\n"),
        "{text}"
    );
    assert!(text.contains("> Hello\n"), "{text}");
    assert_eq!(row, 4);
}

#[test]
fn a_forward_says_what_it_forwards_and_waits_for_whom_to() {
    let (text, row, col) = crate::compose::forward(&to_several());
    assert!(
        text.starts_with("To: \nCc: \nSubject: Fwd: Notes on the engine\n\n"),
        "{text}"
    );
    assert_eq!((row, col), (0, 4));
    assert!(text.contains("Forwarded message"), "{text}");
    assert!(
        text.contains("From: Ada Lovelace <ada@analytical.example>"),
        "{text}"
    );
    assert!(text.contains("\nHello\n"), "{text}");
    assert_eq!(crate::compose::forward_subject("Fwd: x"), "Fwd: x");
    assert_eq!(crate::compose::forward_subject("FW: x"), "FW: x");
}

#[test]
fn a_message_with_files_is_sent_in_parts_each_as_it_was() {
    let draft = crate::compose::Draft {
        to: vec!["a@b.c".into()],
        cc: Vec::new(),
        subject: "Fwd: é".into(),
        attach: Vec::new(),
        body: "Here it is, café.".into(),
    };
    let files = vec![crate::compose::File {
        name: "paper.pdf".into(),
        mime: "application/pdf".into(),
        bytes: b"%PDF-1.4 \xff\x00 binary".to_vec(),
    }];
    let raw = crate::compose::bytes("me@example.com", &draft, None, None, &files);
    assert!(raw.is_ascii(), "every server takes it as it is");
    let message = mail::message(1, raw.clone()).unwrap();
    assert_eq!(message.text.trim(), "Here it is, café.");
    assert_eq!(message.subject, "Fwd: é");
    let files: Vec<(String, String, Vec<u8>)> = mail::attachments(&raw)
        .into_iter()
        .map(|(a, bytes)| (a.name, a.mime, bytes))
        .collect();
    assert_eq!(
        files,
        [(
            "paper.pdf".to_string(),
            "application/pdf".to_string(),
            b"%PDF-1.4 \xff\x00 binary".to_vec()
        )]
    );
}

#[test]
fn reply_forward_and_new_mail_have_their_keys() {
    let mut app = demo();
    press(&mut app, &["<c-n>"]);
    assert!(app.status.contains("new mail"), "{}", app.status);
    // As in Gmail: r replies, a to all, f forwards; C-r and C-f too.
    for (keys, subject) in [
        (["r"], "Subject: Re: "),
        (["<c-r>"], "Subject: Re: "),
        (["a"], "Subject: Re: "),
        (["f"], "Subject: Fwd: "),
        (["<c-f>"], "Subject: Fwd: "),
    ] {
        press(&mut app, &keys);
        let text = app.compose.as_ref().unwrap().buffer.text();
        assert!(text.contains(subject), "{keys:?}: {text}");
        press(&mut app, &["<c-c>", "<c-k>"]);
        assert!(app.compose.is_none());
    }
    let help = {
        press(&mut app, &["?"]);
        app::screen(&mut app, 100, 60)
    };
    for key in ["C-r", "f C-f", "C-n", "R"] {
        assert!(help.contains(key), "{key} in {help}");
    }
}

#[test]
fn c_g_never_loses_the_message_and_c_c_c_k_asks_once_it_is_changed() {
    let mut app = demo();
    press(&mut app, &["c", "ada@analytical.example"]);
    // As often as it is pressed, C-g keeps it, and says how to go on.
    press(&mut app, &["<c-g>", "<c-g>", "<c-g>"]);
    assert!(app.compose.is_some());
    assert!(app.status.contains("C-c C-k discards it"), "{}", app.status);
    // Changed, C-c C-k asks, on the message's own mode line too.
    press(&mut app, &["<c-c>", "<c-k>"]);
    assert!(app.compose.as_ref().unwrap().asking);
    let screen = app::screen(&mut app, 150, 40);
    assert!(screen.contains("kill anyway? (y or n)"), "{screen}");
    // Anything but y goes on writing, the text as it was.
    press(&mut app, &["n"]);
    let text = app.compose.as_ref().unwrap().buffer.text();
    assert!(text.starts_with("To: ada@analytical.example\n"), "{text}");
    assert!(app.status.contains("C-c C-c sends it"), "{}", app.status);
    press(&mut app, &["<c-c>", "<c-k>", "<c-g>"]);
    assert!(app.compose.is_some());
    press(&mut app, &["<c-c>", "<c-k>", "y"]);
    assert!(app.compose.is_none());
    assert!(app.status.contains("discarded"), "{}", app.status);
}

#[test]
fn writing_says_how_it_goes_and_f1_lists_every_key() {
    let mut app = demo();
    press(&mut app, &["c", "ada@analytical.example"]);
    let screen = app::screen(&mut app, 150, 40);
    for said in [
        "C-c C-c send",
        "C-c C-k discard",
        "never losing it",
        "F1: every key",
    ] {
        assert!(screen.contains(said), "{said} in {screen}");
    }
    app.key(KeyEvent::new(KeyCode::F(1), KeyModifiers::NONE));
    assert!(matches!(app.dialog, app::Dialog::Help(_)));
    let screen = app::screen(&mut app, 150, 60);
    assert!(screen.contains("C-c C-k"), "{screen}");
    // Closed, the message is as it was, and keys write in it again.
    press(&mut app, &["<esc>", "!"]);
    assert!(matches!(app.dialog, app::Dialog::Closed));
    let text = app.compose.as_ref().unwrap().buffer.text();
    assert!(text.starts_with("To: ada@analytical.example!"), "{text}");
}

#[test]
fn every_key_shown_is_clicked_as_it_is_pressed() {
    use crate::keymap::key_named;
    let key = |name| app::Target::Key(key_named(name).unwrap());
    let chord = |a, b| app::Target::Chord(key_named(a).unwrap(), key_named(b).unwrap());
    let shown = |app: &mut App, target: app::Target| {
        let _ = app::screen(app, 150, 40);
        app.targets.iter().any(|(_, t)| *t == target)
    };
    let mut app = demo();
    // j/k at the foot is two keys, each its own.
    press(&mut app, &["h"]);
    assert!(shown(&mut app, key("j")) && shown(&mut app, key("k")));
    press(&mut app, &["l"]);
    // Writing: the keys at the foot, under the message, and on its mode line.
    press(&mut app, &["c"]);
    assert!(shown(&mut app, key("F1")));
    assert!(shown(&mut app, key("C-SPC")));
    click_at(&mut app, chord("C-c", "C-a"));
    assert!(matches!(app.dialog, app::Dialog::Attach(_)));
    // On the picker's border: esc.
    click_at(&mut app, key("esc"));
    assert!(matches!(app.dialog, app::Dialog::Closed));
    assert!(app.compose.is_some());
    // Behind it, nothing is clicked.
    let before = app.list.selected();
    click_at(&mut app, app::Target::Popup);
    assert_eq!(app.list.selected(), before);
    // Changed, discarding asks; n and y are there to click.
    press(&mut app, &["ada"]);
    click_at(&mut app, chord("C-c", "C-k"));
    assert!(app.compose.as_ref().unwrap().asking);
    click_at(&mut app, key("n"));
    assert!(app.compose.is_some());
    click_at(&mut app, chord("C-c", "C-k"));
    click_at(&mut app, key("y"));
    assert!(app.compose.is_none(), "{}", app.status);
    // The map's view menu: its rows, and esc on its border.
    press(&mut app, &["<m-g>", ","]);
    click_at(&mut app, key("esc"));
    assert!(matches!(app.dialog, app::Dialog::Closed));
}

#[test]
fn a_forward_carries_the_files_of_what_it_forwards() {
    let mut app = demo();
    let turing = app
        .shown
        .iter()
        .position(|m| m.from == "Alan Turing")
        .unwrap();
    app.list.select(Some(turing));
    press(
        &mut app,
        &["<c-f>", "ada@analytical.example", "<c-c>", "<c-c>"],
    );
    assert!(app.compose.is_none(), "{}", app.status);
    press(&mut app, &["h", "j", "j"]);
    assert_eq!(app.folder_name(), "Sent");
    let sent = app.shown[0].clone();
    assert!(sent.subject.starts_with("Fwd: "), "{}", sent.subject);
    press(&mut app, &["l", "<enter>"]);
    let message = app.message().expect("read").clone();
    assert!(
        message
            .attachments
            .iter()
            .any(|a| a.mime == "application/pdf"),
        "{:?}",
        message.attachments
    );
}

#[test]
fn files_are_picked_a_folder_at_a_time_and_sent_with_the_message() {
    let dir = tempfile::tempdir().unwrap();
    std::fs::create_dir(dir.path().join("papers")).unwrap();
    std::fs::write(dir.path().join("papers/notes.pdf"), b"%PDF-1.4 notes").unwrap();
    std::fs::write(dir.path().join("papers/figure.png"), b"\x89PNG figure").unwrap();
    std::fs::write(dir.path().join("budget.csv"), b"line,cost\n").unwrap();
    let mut app = demo();
    app.attach_dir = Some(dir.path().to_path_buf());
    press(&mut app, &["c", "ada@analytical.example"]);
    // Over the screen, which is still there beneath it.
    let screen = app::screen(&mut app, 150, 40);
    assert!(screen.contains("From you@example.com"), "{screen}");
    assert!(screen.contains("Inbox"), "{screen}");
    press(&mut app, &["<c-c>", "<c-a>"]);
    assert!(
        matches!(app.dialog, app::Dialog::Attach(_)),
        "{}",
        app.status
    );
    let screen = app::screen(&mut app, 150, 40);
    assert!(screen.contains("Attach files"), "{screen}");
    assert!(screen.contains("papers/"), "{screen}");
    // Typing narrows; enter goes into the folder; space marks two.
    press(
        &mut app,
        &["pap", "<enter>", "<space>", "<space>", "<enter>"],
    );
    assert!(matches!(app.dialog, app::Dialog::Closed), "{}", app.status);
    assert_eq!(app.status, "attached 2 files");
    let text = app.compose.as_ref().unwrap().buffer.text();
    assert!(text.contains("/papers/figure.png\n"), "{text}");
    assert!(text.contains("/papers/notes.pdf\n"), "{text}");
    let screen = app::screen(&mut app, 150, 40);
    assert!(screen.contains("14 B"), "the size, beside it: {screen}");
    // Again, from the folder last attached from; a file there once.
    press(&mut app, &["<c-c>", "<c-a>", "<left>", "budget", "<enter>"]);
    press(&mut app, &["<c-c>", "<c-a>", "<enter>"]);
    let text = app.compose.as_ref().unwrap().buffer.text();
    assert_eq!(text.matches("budget.csv").count(), 1, "{text}");
    // A file gone by the time it is sent is said, and nothing is sent.
    press(&mut app, &["<c-c>", "<c-a>", "<esc>"]);
    std::fs::remove_file(dir.path().join("budget.csv")).unwrap();
    press(&mut app, &["<c-c>", "<c-c>"]);
    assert!(app.compose.is_some());
    assert!(app.status.contains("cannot attach"), "{}", app.status);
    std::fs::write(dir.path().join("budget.csv"), b"line,cost\n").unwrap();
    press(&mut app, &["<c-c>", "<c-c>"]);
    assert!(app.compose.is_none(), "{}", app.status);
    press(&mut app, &["h", "j", "j"]);
    assert_eq!(app.folder_name(), "Sent");
    press(&mut app, &["l", "<enter>"]);
    let message = app.message().expect("read").clone();
    let mut names: Vec<&str> = message
        .attachments
        .iter()
        .map(|a| a.name.as_str())
        .collect();
    names.sort();
    assert_eq!(names, ["budget.csv", "figure.png", "notes.pdf"]);
}

#[test]
fn a_forward_s_files_are_attach_lines_that_can_be_let_go() {
    let mut app = demo();
    let turing = app
        .shown
        .iter()
        .position(|m| m.from == "Alan Turing")
        .unwrap();
    app.list.select(Some(turing));
    press(&mut app, &["<c-f>"]);
    let text = app.compose.as_ref().unwrap().buffer.text();
    let lines: Vec<&str> = text
        .lines()
        .filter(|l| l.starts_with("Attach: ") && l.ends_with(" (forwarded)"))
        .collect();
    assert_eq!(lines.len(), 2, "{text}");
    // The lines killed, the files stay behind.
    let row = text.lines().position(|l| l == lines[0]).unwrap();
    app.compose.as_mut().unwrap().buffer.place(row, 0);
    press(&mut app, &["<c-k>", "<c-k>", "<c-k>", "<c-k>"]);
    let compose = app.compose.as_mut().unwrap();
    compose.buffer.place(0, 4);
    press(&mut app, &["ada@analytical.example", "<c-c>", "<c-c>"]);
    assert!(app.compose.is_none(), "{}", app.status);
    press(&mut app, &["h", "j", "j", "l", "<enter>"]);
    let message = app.message().expect("read").clone();
    assert!(message.subject.starts_with("Fwd: "), "{}", message.subject);
    assert!(message.attachments.is_empty(), "{:?}", message.attachments);
}

#[test]
fn files_dropped_on_the_terminal_are_attached_and_other_text_written() {
    let dir = tempfile::tempdir().unwrap();
    let spaced = dir.path().join("a draft.txt");
    std::fs::write(&spaced, b"draft").unwrap();
    let plain = dir.path().join("plain.txt");
    std::fs::write(&plain, b"plain").unwrap();
    let mut app = demo();
    press(&mut app, &["c"]);
    // As terminals paste dropped files: escaped, quoted, or as addresses.
    let escaped = spaced.display().to_string().replace(' ', "\\ ");
    app.paste(&format!("{escaped} '{}'", plain.display()));
    let text = app.compose.as_ref().unwrap().buffer.text();
    assert!(text.contains("a draft.txt\n"), "{text}");
    assert!(text.contains("plain.txt\n"), "{text}");
    let address = format!(
        "file://{}",
        spaced.display().to_string().replace(' ', "%20")
    );
    app.paste(&address);
    assert_eq!(app.status, "attached already");
    // Anything else is written where the cursor is.
    app.paste("Dear Ada,\nthe engine");
    let text = app.compose.as_ref().unwrap().buffer.text();
    assert!(text.contains("Dear Ada,\nthe engine"), "{text}");
    assert!(!text.contains("Attach: Dear"), "{text}");
}

#[test]
fn a_capital_said_as_its_letter_with_shift_is_the_capital() {
    let shift = |c| KeyEvent::new(KeyCode::Char(c), KeyModifiers::SHIFT);
    let mut app = demo();
    // A, as kitty's protocol may say it: the account form.
    app.key(shift('a'));
    assert!(app.dialog.form().is_some());
    press(&mut app, &["<esc>"]);
    // Written, it is the capital too.
    press(&mut app, &["c"]);
    app.key(shift('h'));
    let text = app.compose.as_ref().unwrap().buffer.text();
    assert!(text.starts_with("To: H"), "{text}");
}

#[test]
fn an_account_is_renamed_in_its_file_its_comments_kept() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("accounts.toml");
    std::fs::write(
        &path,
        "# my mail\n[[account]]\nname = \"Home\" # the old one\nemail = \"a@b.c\"\nhost = \"imap.b.c\"\n\
         [[account]]\nname = \"Work\"\nemail = \"w@b.c\"\nhost = \"imap.b.c\"\n",
    )
    .unwrap();
    crate::config::rename(&path, "home", "Personal").unwrap();
    let file = std::fs::read_to_string(&path).unwrap();
    assert!(file.starts_with("# my mail\n"), "{file}");
    assert!(file.contains("name = \"Personal\""), "{file}");
    assert!(
        crate::config::rename(&path, "Personal", "work").is_err(),
        "a name twice"
    );
    assert!(crate::config::rename(&path, "Nobody", "X").is_err());
    assert!(
        crate::config::rename(&path, "Personal", " ").is_err(),
        "a name is something"
    );
}

#[test]
fn an_account_is_renamed_from_the_sidebar() {
    let (account, _) = server();
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("accounts.toml");
    let mut app = empty_app(path.clone());
    fill_form(&mut app, account.port.unwrap(), "secret");
    assert!(app.dialog.form().is_none());
    press(&mut app, &["h", "R"]);
    assert!(app::screen(&mut app, 120, 40).contains("Rename"));
    press(&mut app, &["<c-u>", "Personal", "<enter>"]);
    assert_eq!(app.accounts[0].name, "Personal", "{}", app.status);
    assert_eq!(crate::config::load(&path).unwrap()[0].name, "Personal");
    // Esc leaves it as it is, and so does a click away from it.
    press(&mut app, &["R", "<c-u>", "X", "<esc>"]);
    assert_eq!(app.accounts[0].name, "Personal");
    press(&mut app, &["R"]);
    click_at(&mut app, app::Target::Side);
    assert!(!matches!(app.dialog, app::Dialog::Rename(..)));
    assert_eq!(app.accounts[0].name, "Personal");
}

#[test]
fn an_account_the_file_names_otherwise_is_not_said_to_be_renamed() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("accounts.toml");
    std::fs::write(
        &path,
        "account = [{ name = \"Home\", email = \"a@b.c\", host = \"imap.b.c\" }]\n",
    )
    .unwrap();
    assert!(crate::config::rename(&path, "Home", "Personal").is_err());
}

#[test]
fn a_reply_to_all_of_your_own_message_is_to_whom_it_went() {
    let raw = b"From: You <you@example.com>\r\n\
        To: Bob <bob@example.com>, Carol <carol@example.com>\r\n\
        Cc: Dan <dan@example.com>\r\nSubject: Plans\r\n\r\nHi\r\n"
        .to_vec();
    let message = mail::message(1, raw).unwrap();
    let (text, _) = crate::compose::reply_all(&message, "you@example.com");
    assert!(
        text.starts_with(
            "To: Bob <bob@example.com>, Carol <carol@example.com>\nCc: Dan <dan@example.com>\n"
        ),
        "{text}"
    );
}

#[test]
fn a_reply_is_to_every_address_the_message_asks_replies_go_to() {
    let raw = b"From: Ada <ada@analytical.example>\r\n\
        Reply-To: a@x.example, b@y.example\r\nTo: you@example.com\r\n\
        Subject: Plans\r\n\r\nHi\r\n"
        .to_vec();
    let message = mail::message(1, raw).unwrap();
    let (text, _) = crate::compose::reply(&message);
    assert!(text.starts_with("To: a@x.example, b@y.example\n"), "{text}");
}

#[test]
fn a_forwards_files_keep_each_its_own_type() {
    let raw = b"From: Ada <ada@analytical.example>\r\nTo: you@example.com\r\n\
        Subject: Pictures\r\nMIME-Version: 1.0\r\n\
        Content-Type: multipart/mixed; boundary=b\r\n\r\n\
        --b\r\nContent-Type: text/plain\r\n\r\nTwo\r\n\
        --b\r\nContent-Type: image/png\r\n\
        Content-Disposition: attachment; filename=same\r\n\r\npng\r\n\
        --b\r\nContent-Type: image/jpeg\r\n\
        Content-Disposition: attachment; filename=same\r\n\r\njpg\r\n\
        --b--\r\n"
        .to_vec();
    let message = mail::message(1, raw).unwrap();
    let files = crate::compose::files(&message);
    let types: Vec<&str> = files.iter().map(|f| f.mime.as_str()).collect();
    assert_eq!(types, ["image/png", "image/jpeg"]);
    assert!(files.iter().all(|f| !f.mime.is_empty()));
}

// ---- the mark, the region, and searching as you type, in compose ----

/// Press `keys`, as `app::keys` names them, in `buffer`; what the last did.
fn edit_keys(buffer: &mut crate::edit::Buffer, keys: &[&str]) -> crate::edit::Done {
    let mut prefix = false;
    let mut done = crate::edit::Done::Stay;
    for key in keys.iter().flat_map(|k| app::keys(k)) {
        done = crate::edit::handle(buffer, &mut prefix, key);
    }
    done
}

#[test]
fn the_region_from_the_mark_is_cut_with_c_w_and_copied_with_m_w() {
    use crate::edit::{Buffer, Done};
    let mut buffer = Buffer::from_text("hello world");
    edit_keys(&mut buffer, &["<c- >"]);
    assert_eq!(buffer.region(), Some(((0, 0), (0, 0))));
    edit_keys(&mut buffer, &["<m-f>"]);
    assert_eq!(buffer.region(), Some(((0, 0), (0, 5))));
    edit_keys(&mut buffer, &["<c-w>"]);
    assert_eq!(buffer.text(), " world\n");
    assert_eq!(buffer.region(), None, "the mark goes with the region");
    edit_keys(&mut buffer, &["<c-e>", "<c-y>"]);
    assert_eq!(buffer.text(), " worldhello\n");

    // M-w copies: the text stays, and C-y puts the copy at point.
    let mut buffer = Buffer::from_text("abc");
    edit_keys(&mut buffer, &["<c-@>", "<c-e>"]);
    edit_keys(&mut buffer, &["<m-w>"]);
    assert_eq!(buffer.text(), "abc\n");
    assert_eq!(buffer.region(), None);
    edit_keys(&mut buffer, &["<c-y>"]);
    assert_eq!(buffer.text(), "abcabc\n");

    // Across lines.
    let mut buffer = Buffer::from_text("one\ntwo\nthree");
    edit_keys(&mut buffer, &["<c-f>", "<c- >", "<c-n>", "<c-n>", "<c-w>"]);
    assert_eq!(buffer.text(), "ohree\n");

    // With no mark, C-w cuts nothing.
    let mut buffer = Buffer::from_text("keep");
    edit_keys(&mut buffer, &["<c-e>", "<c-w>"]);
    assert_eq!(buffer.text(), "keep\n");

    // C-g lets the mark go, and then only says it quit: never the message.
    let mut buffer = Buffer::from_text("x");
    assert_eq!(edit_keys(&mut buffer, &["<c- >", "<c-g>"]), Done::Stay);
    assert_eq!(buffer.region(), None);
    assert_eq!(edit_keys(&mut buffer, &["<c-g>"]), Done::Quit);
    assert_eq!(edit_keys(&mut buffer, &["<c-c>", "<c-k>"]), Done::Cancel);
}

#[test]
fn c_s_and_c_r_search_as_you_type() {
    use crate::edit::{Buffer, Done};
    let mut buffer = Buffer::from_text("one two\none Two");
    edit_keys(&mut buffer, &["<c-s>", "t", "w", "o"]);
    assert_eq!((buffer.row(), buffer.col()), (0, 7), "after the first");
    assert_eq!(buffer.searching(), Some("I-search: two".to_string()));
    edit_keys(&mut buffer, &["<c-s>"]);
    assert_eq!(
        (buffer.row(), buffer.col()),
        (1, 7),
        "case is ignored in lower case"
    );
    // ⏎ ends the search where it is; what is typed then is typed.
    edit_keys(&mut buffer, &["<enter>"]);
    assert_eq!(buffer.searching(), None);
    assert_eq!((buffer.row(), buffer.col()), (1, 7));

    // A capital is looked for as it is; failing, it says so.
    let mut buffer = Buffer::from_text("one two\none Two");
    edit_keys(&mut buffer, &["<c-s>", "T", "w", "o"]);
    assert_eq!((buffer.row(), buffer.col()), (1, 7));
    edit_keys(&mut buffer, &["<c-s>"]);
    assert_eq!(
        buffer.searching(),
        Some("Failing I-search: Two".to_string())
    );
    // ⌫ takes a letter back, searching again from where it began.
    edit_keys(
        &mut buffer,
        &["<backspace>", "<backspace>", "<backspace>", "o"],
    );
    assert_eq!((buffer.row(), buffer.col()), (0, 1));

    // C-g goes back to where the search began.
    let mut buffer = Buffer::from_text("one two");
    edit_keys(&mut buffer, &["<c-e>", "<c-r>", "o", "n"]);
    assert_eq!(
        (buffer.row(), buffer.col()),
        (0, 0),
        "C-r lands at its start"
    );
    assert_eq!(edit_keys(&mut buffer, &["<c-g>"]), Done::Stay);
    assert_eq!(buffer.searching(), None);
    assert_eq!((buffer.row(), buffer.col()), (0, 7));

    // Another key ends the search and does what it does.
    let mut buffer = Buffer::from_text("abc");
    edit_keys(&mut buffer, &["<c-s>", "b", "<c-a>"]);
    assert_eq!((buffer.searching(), buffer.col()), (None, 0));
}

#[test]
fn the_search_shows_in_the_compose_panes_mode_line() {
    let mut app = demo();
    press(&mut app, &["c", "<c-s>", "S", "u", "b"]);
    let screen = app::screen(&mut app, 120, 40);
    assert!(screen.contains("I-search: Sub"), "{screen}");
}

#[test]
fn the_search_and_the_region_keep_to_emacs_in_the_corners() {
    use crate::edit::{Buffer, Done, handle};
    let shout = |c: char| {
        KeyEvent::new(
            KeyCode::Char(c),
            KeyModifiers::CONTROL | KeyModifiers::SHIFT,
        )
    };
    // C-G, as some terminals say C-g, ends the search, not the message.
    let mut buffer = Buffer::from_text("one two");
    edit_keys(&mut buffer, &["<c-s>", "t"]);
    assert_eq!(handle(&mut buffer, &mut false, shout('G')), Done::Stay);
    assert_eq!((buffer.searching(), buffer.col()), (None, 0));
    // Turned about, the search lands on the other end of what it found.
    let mut buffer = Buffer::from_text("one two");
    edit_keys(&mut buffer, &["<c-s>", "t", "w", "o"]);
    assert_eq!(buffer.col(), 7);
    edit_keys(&mut buffer, &["<c-r>"]);
    assert_eq!(
        (
            buffer.col(),
            buffer.searching().unwrap().starts_with("I-search")
        ),
        (4, true)
    );
    // Going back, what is found ends at or before point.
    let mut buffer = Buffer::from_text("abcdef");
    edit_keys(&mut buffer, &["<c-f>", "<c-r>", "a", "b"]);
    assert!(buffer.searching().unwrap().starts_with("Failing"));
    // A search taken back to nothing leaves C-n aiming where it began.
    let mut buffer = Buffer::from_text("ab\nabcdefgh\nabcdefgh");
    edit_keys(
        &mut buffer,
        &["<c-s>", "h", "<backspace>", "<enter>", "<c-n>"],
    );
    assert_eq!((buffer.row(), buffer.col()), (1, 0));
    // After C-w, C-n aims from where the cut left point.
    let mut buffer = Buffer::from_text("hello world\nabcdefghijkl");
    edit_keys(&mut buffer, &["<c- >", "<m-f>", "<c-w>", "<c-n>"]);
    assert_eq!((buffer.row(), buffer.col()), (1, 0));
    // C-w then C-k: one kill, put back together.
    let mut buffer = Buffer::from_text("abc def");
    edit_keys(
        &mut buffer,
        &["<c- >", "<c-f>", "<c-f>", "<c-w>", "<c-k>", "<c-y>"],
    );
    assert_eq!(buffer.text(), "abc def\n");
}

// ---- the key table ----

#[test]
fn every_key_in_the_table_is_read_and_bound_once() {
    use crate::keymap::{KEYS, key_named};
    use ratatui::crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
    let mut seen = std::collections::HashSet::new();
    for (name, act) in KEYS.iter().flat_map(|b| b.keys) {
        let key = key_named(name).unwrap_or_else(|| panic!("{name} is no key"));
        assert_eq!(crate::keymap::act(key), Some(*act), "{name}");
        assert!(
            seen.insert(crate::keymap::chord(key)),
            "{name} is bound twice"
        );
    }
    // Ctrl and Shift, as terminals say it or not, is no key of ours, as
    // kitty keeps ctrl+shift for itself: not C-r's.
    let ctrl_shift = KeyModifiers::CONTROL | KeyModifiers::SHIFT;
    for key in [
        KeyEvent::new(KeyCode::Char('r'), ctrl_shift),
        KeyEvent::new(KeyCode::Char('R'), ctrl_shift),
        KeyEvent::new(KeyCode::Char('R'), KeyModifiers::CONTROL),
    ] {
        assert_eq!(crate::keymap::act(key), None);
    }
    assert_eq!(
        crate::keymap::act(KeyEvent::new(KeyCode::Char('G'), KeyModifiers::SHIFT)),
        crate::keymap::act(key_named("end").unwrap()),
    );
    assert_eq!(
        crate::keymap::act(KeyEvent::new(KeyCode::Char('e'), KeyModifiers::CONTROL)),
        None,
        "C-e is not e"
    );
}

#[test]
fn the_readme_names_every_key_the_help_does() {
    let readme = include_str!("../README.md");
    let said = crate::keymap::KEYS
        .iter()
        .filter(|b| !b.help.is_empty() || !b.foot.is_empty());
    for b in said {
        for key in b.name.split(' ') {
            assert!(readme.contains(&format!("`{key}`")), "`{key}` in README");
        }
    }
}

#[test]
fn the_foot_and_the_help_come_from_the_table() {
    let mut app = demo();
    let foot = app::screen(&mut app, 120, 30);
    for h in crate::keymap::foot(crate::keymap::Pane::List) {
        let hint = format!(" {} {}", h.key, h.what);
        assert!(foot.contains(&hint), "{hint} in {foot}");
    }
    press(&mut app, &["?"]);
    let help = app::screen(&mut app, 100, 80);
    for (name, _) in crate::keymap::help() {
        assert!(help.contains(name), "{name} in {help}");
    }
}

#[test]
fn the_wheel_moves_in_the_dialog_open_and_not_behind_it() {
    let mut app = demo();
    let turing = app
        .shown
        .iter()
        .position(|m| m.from == "Alan Turing")
        .unwrap();
    app.list.select(Some(turing));
    press(&mut app, &["<enter>", "o"]);
    let _ = app::screen(&mut app, 120, 40);
    for _ in 0..3 {
        app.mouse(mouse(MouseEventKind::ScrollDown, 1, 1));
    }
    assert_eq!(
        app.dialog.files().expect("listed").at,
        2,
        "the last of three"
    );
    press(&mut app, &["<esc>"]);

    // A question asked, the list behind it stays where it is.
    app.dialog = app::Dialog::Rename(0, "Demo".to_string());
    let before = app.list.selected();
    let _ = app::screen(&mut app, 120, 40);
    app.mouse(mouse(MouseEventKind::ScrollDown, 60, 10));
    assert_eq!(app.list.selected(), before);
    assert!(matches!(app.dialog, app::Dialog::Rename(..)));
}

#[test]
fn space_pages_the_message_and_leaves_the_sidebar_alone() {
    let mut app = demo();
    press(&mut app, &["h"]);
    let (row, picked) = (app.row, app.list.selected());
    press(&mut app, &[" "]);
    assert_eq!(app.row, row, "space does not move the folders");
    assert_eq!(app.list.selected(), picked, "nor the messages");
    press(&mut app, &["<pgdn>"]);
    assert_ne!(app.row, row, "the page key does");
}

#[test]
fn a_narrow_foot_leaves_hints_out_but_help_and_quit() {
    let mut app = demo();
    let screen = app::screen(&mut app, 40, 20);
    assert!(
        screen.contains(" ? help") && screen.contains(" q quit"),
        "{screen}"
    );
    assert!(!screen.contains("new mail"), "{screen}");
    // Writing, the foot fits in its two lines too.
    press(&mut app, &["c"]);
    let screen = app::screen(&mut app, 40, 20);
    let lines: Vec<&str> = screen.lines().collect();
    assert!(lines[lines.len() - 3].contains("Compose"), "{screen}");
    assert!(lines[lines.len() - 2].contains("send"), "{screen}");
}

// ---- the colours ----

/// WCAG's contrast of two colours, 1 to 21; the terminal's own dark
/// ground as the theme takes it.
fn contrast(a: ratatui::style::Color, b: ratatui::style::Color) -> f64 {
    use ratatui::style::Color;
    let luminance = |c: Color| {
        let (r, g, b) = match c {
            Color::Rgb(r, g, b) => (r, g, b),
            Color::Reset => crate::theme::TERMINAL_DARK,
            other => panic!("{other:?} is not a colour to measure"),
        };
        let lin = |v: u8| {
            let v = f64::from(v) / 255.0;
            if v <= 0.04045 {
                v / 12.92
            } else {
                ((v + 0.055) / 1.055).powf(2.4)
            }
        };
        0.2126 * lin(r) + 0.7152 * lin(g) + 0.0722 * lin(b)
    };
    let (x, y) = (luminance(a), luminance(b));
    (x.max(y) + 0.05) / (x.min(y) + 0.05)
}

#[test]
fn every_colour_reads_on_every_ground_it_is_drawn_on() {
    use crate::theme::{Name, Theme};
    use clap::ValueEnum;
    for &name in Name::value_variants() {
        let t = Theme::of(name);
        let accounts = t.accounts().iter().map(|&c| ("account", c, 4.5));
        let inks = [
            ("title", t.title, 7.0),
            ("text", t.text, 7.0),
            ("soft", t.soft, 4.5),
            ("dim", t.dim, 4.5),
            // Signatures and notes are faint too: text, not only icons.
            ("faint", t.faint, 4.5),
            ("accent", t.accent, 4.5),
            ("flag", t.flag, 4.5),
            ("error", t.error, 4.5),
        ];
        let grounds = [
            ("page", t.page),
            ("side", t.side),
            ("field", t.field),
            ("focus", t.focus),
            ("row", t.row),
        ];
        for (ink, colour, need) in inks.into_iter().chain(accounts) {
            for (ground, behind) in grounds {
                let got = contrast(colour, behind);
                assert!(
                    got >= need,
                    "{name:?}: {ink} on {ground} is {got:.1}:1, under {need}:1"
                );
            }
        }
        // Labels drawn on a colour: the button pressed with ⏎, and the
        // mode line's name of the message being written.
        for (what, ink, behind) in [
            ("a button", t.on_accent, t.accent),
            ("Compose", t.side, t.title),
        ] {
            let got = contrast(ink, behind);
            assert!(got >= 4.5, "{name:?}: {what} reads {got:.1}:1");
        }
        // Each step down reads less than the one above it.
        let steps = [t.title, t.text, t.soft, t.dim, t.faint].map(|c| contrast(c, t.page));
        assert!(steps.is_sorted_by(|a, b| a > b), "{name:?}: {steps:?}");
        // Grounds that must be told apart: the panes, the picked row in
        // the pane with the focus and in the others, and a rule under it.
        for (what, a, b) in [
            ("side and page", t.side, t.page),
            ("side and row", t.side, t.row),
            ("focus and row", t.focus, t.row),
            ("rule and focus", t.rule, t.focus),
            ("rule and page", t.rule, t.page),
        ] {
            let got = contrast(a, b);
            assert!(got >= 1.12, "{name:?}: {what} are {got:.2}:1 apart");
        }
    }
}

#[test]
fn unread_a_flag_and_the_accounts_are_told_apart_by_hue() {
    // Blue for unread, orange for a flag, as every common colour blindness
    // tells them apart; the accounts in neither.
    use crate::theme::{Name, Theme};
    use clap::ValueEnum;
    use ratatui::style::Color;
    let hue = |c: Color| {
        let Color::Rgb(r, g, b) = c else {
            panic!("{c:?} is not a colour");
        };
        let (r, g, b) = (f64::from(r), f64::from(g), f64::from(b));
        let (max, min) = (r.max(g).max(b), r.min(g).min(b));
        let h = if max == r {
            (g - b) / (max - min)
        } else if max == g {
            2.0 + (b - r) / (max - min)
        } else {
            4.0 + (r - g) / (max - min)
        };
        (h * 60.0).rem_euclid(360.0)
    };
    let apart = |a: f64, b: f64| {
        let d = (a - b).abs();
        d.min(360.0 - d)
    };
    for &name in Name::value_variants() {
        let t = Theme::of(name);
        let (unread, flag) = (hue(t.accent), hue(t.flag));
        assert!(
            (190.0..230.0).contains(&unread),
            "{name:?}: unread {unread}"
        );
        assert!((20.0..40.0).contains(&flag), "{name:?}: flag {flag}");
        for (i, &c) in t.accounts().iter().enumerate() {
            let h = hue(c);
            for (what, other) in [("unread", unread), ("flag", flag)] {
                let d = apart(h, other);
                assert!(d >= 30.0, "{name:?}: account {i} is {d} from {what}");
            }
        }
    }
}

// ---- the map's index: its journals and its key ----

use crate::index::{self, Cached, Index, Kept, Lens};

/// A record of the account `a@b.c`, as fetched from INBOX, known by `id`.
fn record(id: &str, subject: &str) -> index::Record {
    let indexed = index::record::Indexed {
        uid: 1,
        size: 100,
        raw: raw(
            "Ada Lovelace <ada@analytical.example>",
            subject,
            "",
            "Hello.\r\n",
        ),
        ..Default::default()
    };
    let mut r = index::Record::of("a@b.c", "INBOX", 7, &indexed, 1_000);
    r.key.id = id.to_string();
    r
}

fn identity() -> age::x25519::Identity {
    age::x25519::Identity::generate()
}

#[test]
fn the_journals_give_back_what_was_written() {
    let dir = tempfile::tempdir().unwrap();
    let id = identity();
    let (a, b) = (record("a", "Notes on the engine"), record("b", "Photo 51"));
    {
        let mut index = Index::open_with(dir.path(), &id);
        index
            .cache(vec![
                Cached::Upsert(Box::new(a.clone())),
                Cached::Upsert(Box::new(b.clone())),
                Cached::Flags {
                    key: b.key.clone(),
                    flags: mail::Flags {
                        seen: true,
                        ..Default::default()
                    },
                },
            ])
            .unwrap();
        index
            .keep(vec![
                Kept::Label {
                    key: a.key.clone(),
                    label: "Grant".into(),
                    on: true,
                },
                Kept::Layout {
                    lens: Lens::Both,
                    at: vec![(a.key.clone(), 1.5, -2.0)],
                },
                Kept::Looked { at: 42 },
            ])
            .unwrap();
        index.flush().unwrap();
    }
    let index = Index::open_with(dir.path(), &id);
    assert!(index.fault.is_none(), "{:?}", index.fault);
    assert_eq!(index.records.len(), 2);
    assert_eq!(index.records[&a.key], a);
    assert!(index.records[&b.key].flags.seen);
    assert_eq!(index.keep.labels[&a.key], ["Grant"]);
    assert_eq!(index.keep.layouts[&Lens::Both][&a.key], (1.5, -2.0));
    assert_eq!(index.keep.looked, 42);
    // Encrypted: nothing of the mail is in the files.
    for entry in std::fs::read_dir(dir.path()).unwrap() {
        let bytes = std::fs::read(entry.unwrap().path()).unwrap();
        let text = String::from_utf8_lossy(&bytes);
        assert!(!text.contains("Photo") && !text.contains("Grant"));
        assert!(text.starts_with("age-encryption.org/v1"));
    }
    // Another key reads nothing, and says so.
    let other = Index::open_with(dir.path(), &identity());
    assert!(other.records.is_empty());
    assert!(other.keep_fault().is_some());
}

#[test]
fn new_mail_is_written_with_the_public_key_alone() {
    let dir = tempfile::tempdir().unwrap();
    let id = identity();
    let a = record("a", "Notes on the engine");
    {
        let mut index = Index::open_with(dir.path(), &id);
        index
            .cache(vec![Cached::Upsert(Box::new(a.clone()))])
            .unwrap();
    }
    let b = record("b", "Photo 51");
    {
        let mut writer = Index::writer(dir.path(), id.to_public());
        writer
            .cache(vec![Cached::Upsert(Box::new(b.clone()))])
            .unwrap();
        writer.flush().unwrap();
    }
    let index = Index::open_with(dir.path(), &id);
    assert_eq!(index.records.len(), 2, "{:?}", index.fault);
    assert_eq!(index.records[&b.key].subject, "Photo 51");
}

#[test]
fn a_base_written_and_its_logs_not_yet_removed_is_read_once() {
    use crate::index::journal::Journal;
    let dir = tempfile::tempdir().unwrap();
    let aside = tempfile::tempdir().unwrap();
    let id = identity();
    let open = || Journal::<u32>::open(dir.path(), "cache", id.to_public(), Some(&id));
    {
        let (mut j, _) = open();
        for n in 0..3 {
            j.push(n).unwrap();
            j.flush().unwrap();
        }
        assert_eq!(j.logs(), 3);
    }
    let copy = |from: &std::path::Path, to: &std::path::Path| {
        for entry in std::fs::read_dir(from).unwrap() {
            let path = entry.unwrap().path();
            std::fs::copy(&path, to.join(path.file_name().unwrap())).unwrap();
        }
    };
    copy(dir.path(), aside.path());
    {
        let (mut j, all) = open();
        assert_eq!(all, [0, 1, 2]);
        j.compact(&all).unwrap();
        assert_eq!(j.logs(), 0);
    }
    // As if letterbox stopped once the base was written, before the files
    // it takes the place of were removed.
    copy(aside.path(), dir.path());
    let (mut j, all) = open();
    assert_eq!(all, [0, 1, 2], "the new base alone");
    j.push(3).unwrap();
    j.flush().unwrap();
    drop(j);
    assert_eq!(open().1, [0, 1, 2, 3]);
}

#[test]
fn a_journal_with_many_logs_is_written_again_as_one_base() {
    let dir = tempfile::tempdir().unwrap();
    let id = identity();
    let many = crate::index::journal::LOGS + 1;
    {
        let mut index = Index::open_with(dir.path(), &id);
        for n in 0..many {
            let r = record(&n.to_string(), "Photo 51");
            index.cache(vec![Cached::Upsert(Box::new(r))]).unwrap();
            index.flush().unwrap();
        }
    }
    let files = || std::fs::read_dir(dir.path()).unwrap().count();
    assert_eq!(files(), many);
    let index = Index::open_with(dir.path(), &id);
    assert_eq!(index.records.len(), many);
    assert_eq!(files(), 1, "one base, the cache's; keep has nothing");
    drop(index);
    assert_eq!(Index::open_with(dir.path(), &id).records.len(), many);
}

/// Spoil the first file of `name` in `dir`, as a disk might; where it is.
fn spoil(dir: &std::path::Path, name: &str) -> PathBuf {
    let path = std::fs::read_dir(dir)
        .unwrap()
        .map(|e| e.unwrap().path())
        .find(|p| p.file_name().unwrap().to_string_lossy().starts_with(name))
        .unwrap();
    let mut bytes = std::fs::read(&path).unwrap();
    let at = bytes.len() - 5;
    bytes[at] ^= 0xff;
    std::fs::write(&path, &bytes).unwrap();
    path
}

#[test]
fn a_cache_that_cannot_be_read_is_begun_again() {
    let dir = tempfile::tempdir().unwrap();
    let id = identity();
    {
        let mut index = Index::open_with(dir.path(), &id);
        let r = record("a", "Photo 51");
        index.cache(vec![Cached::Upsert(Box::new(r))]).unwrap();
    }
    spoil(dir.path(), "cache.");
    let index = Index::open_with(dir.path(), &id);
    assert!(index.records.is_empty());
    let fault = index.fault.clone().unwrap();
    assert!(fault.contains("made again from the servers"), "{fault}");
    drop(index);
    let index = Index::open_with(dir.path(), &id);
    assert!(index.fault.is_none(), "{:?}", index.fault);
}

#[test]
fn a_keep_that_cannot_be_read_is_not_written_over() {
    let dir = tempfile::tempdir().unwrap();
    let id = identity();
    let key = record("a", "Photo 51").key;
    let label = |on| Kept::Label {
        key: key.clone(),
        label: "Grant".into(),
        on,
    };
    {
        let mut index = Index::open_with(dir.path(), &id);
        index.keep(vec![label(true)]).unwrap();
    }
    let path = spoil(dir.path(), "keep.");
    let before = std::fs::read(&path).unwrap();
    let mut index = Index::open_with(dir.path(), &id);
    assert!(index.keep_fault().is_some());
    let e = index.keep(vec![label(false)]).unwrap_err();
    assert!(e.to_string().contains("cannot read"), "{e}");
    index.flush().unwrap();
    drop(index);
    assert_eq!(std::fs::read(&path).unwrap(), before);
    assert_eq!(std::fs::read_dir(dir.path()).unwrap().count(), 1);
}

/// Where a test keeps the map's key: here, the system's store left alone.
static KEPT_KEY: Mutex<Option<(Keeper, String)>> = Mutex::new(None);

fn keep_key(keeper: Keeper, host: &str, user: &str, secret: &str) -> anyhow::Result<()> {
    assert_eq!((host, user), ("map", "identity"));
    *KEPT_KEY.lock().unwrap() = Some((keeper, secret.to_string()));
    Ok(())
}

fn find_key(keeper: Keeper, _: &str, _: &str) -> anyhow::Result<String> {
    match KEPT_KEY.lock().unwrap().clone() {
        Some((k, secret)) if k == keeper => Ok(secret),
        _ => anyhow::bail!("no such key"),
    }
}

fn refuse_key(_: Keeper, _: &str, _: &str, _: &str) -> anyhow::Result<()> {
    anyhow::bail!("the store is closed")
}

#[test]
fn the_maps_key_is_kept_before_it_is_named() {
    use crate::index::key;
    let dir = tempfile::tempdir().unwrap();
    // Nowhere to keep it: no map.toml names a key that is not kept.
    let e = Index::open(dir.path(), || vec![Keeper::Keyring], find_key, refuse_key)
        .err()
        .unwrap();
    assert!(e.contains("the store is closed"), "{e}");
    assert!(key::card(dir.path()).unwrap().is_none());

    let r = record("a", "Photo 51");
    {
        let mut index =
            Index::open(dir.path(), || vec![Keeper::Keyring], find_key, keep_key).unwrap();
        index
            .cache(vec![Cached::Upsert(Box::new(r.clone()))])
            .unwrap();
    }
    let card = key::card(dir.path()).unwrap().unwrap();
    assert_eq!((card.version, card.keeper), (key::VERSION, Keeper::Keyring));
    let secret = KEPT_KEY.lock().unwrap().clone().unwrap().1;
    assert!(secret.starts_with("AGE-SECRET-KEY-"));
    let written = std::fs::read_to_string(dir.path().join("map.toml")).unwrap();
    assert!(!written.contains(&secret), "{written}");
    // Opened again, the key is found where the card says, and reads it.
    let index = Index::open(dir.path(), || vec![Keeper::Vault], find_key, refuse_key).unwrap();
    assert_eq!(index.records[&r.key].subject, "Photo 51");
    let found = key::identity(&card, find_key).unwrap();
    assert!(key::export(&found).contains(&secret));
}

// ---- the map's sync ----

/// One pass of the sync over `store`, into `index`, as at the time `now`.
fn sync_at(store: &mut dyn Store, index: &Mutex<Index>, now: i64) -> index::sync::Passed {
    let stop = std::sync::atomic::AtomicBool::new(false);
    index::sync::pass(store, index, now, &stop, &mut |_| {}).unwrap()
}

fn sync_once(store: &mut dyn Store, index: &Mutex<Index>) -> index::sync::Passed {
    sync_at(store, index, 1_000_000)
}

/// The folders the index places each of `account`'s messages in, by
/// subject.
fn places(index: &Mutex<Index>) -> std::collections::BTreeMap<String, Vec<String>> {
    index
        .lock()
        .unwrap()
        .records
        .values()
        .map(|r| {
            (
                r.subject.clone(),
                r.places.iter().map(|p| p.folder.clone()).collect(),
            )
        })
        .collect()
}

/// The UID sets of the messages fetched whole for the index, as the
/// server was asked for them, in turn.
fn index_fetches(boxes: &Boxes) -> Vec<String> {
    boxes
        .lock()
        .unwrap()
        .said
        .iter()
        .filter(|s| s.starts_with("UID FETCH") && s.contains("LIST-ID"))
        .map(|s| s.split(' ').nth(2).unwrap_or_default().to_string())
        .collect()
}

#[test]
fn the_sync_reads_the_inbox_first_newest_first_a_hundred_at_a_time() {
    let (account, boxes) = serve_boxes(boxes_in(Mode::Keywords));
    let mut uids = Vec::new();
    for n in 0..150 {
        uids.push(add_listed(&boxes, "INBOX", &[], &format!("Build {n}")));
    }
    add_listed(&boxes, "Archive", &["\\Seen"], "Old news");
    add_listed(&boxes, "Drafts", &[], "Unfinished");
    add_listed(&boxes, "Deleted Items", &[], "Thrown away");
    let index = Mutex::new(Index::memory());
    let mut imap = Imap::new(account);
    let passed = sync_once(&mut imap, &index);
    assert!(passed.complete);
    let fetches = index_fetches(&boxes);
    let newest = crate::imap::uid_set(&uids[50..]);
    assert_eq!(fetches[0], newest, "the newest hundred of the Inbox first");
    assert_eq!(fetches.len(), 3, "{fetches:?}");
    let at = places(&index);
    assert_eq!(at.len(), 3 + 150 + 1, "{:?}", at.keys());
    assert_eq!(at["Old news"], ["Archive"]);
    assert_eq!(at["Photo 51"], ["INBOX"]);
    assert!(!at.contains_key("Unfinished") && !at.contains_key("Thrown away"));

    // Again, nothing new: nothing is fetched whole.
    sync_once(&mut imap, &index);
    assert_eq!(index_fetches(&boxes).len(), 3);
}

#[test]
fn new_mail_and_mail_gone_are_seen_by_the_next_pass() {
    let (account, boxes) = serve_boxes(boxes_in(Mode::Keywords));
    let index = Mutex::new(Index::memory());
    let mut imap = Imap::new(account);
    sync_once(&mut imap, &index);
    let new = add_listed(&boxes, "INBOX", &[], "PR #38 merged");
    boxes.lock().unwrap()[0].letters.retain(|l| l.uid != 11);
    sync_at(&mut imap, &index, 2_000_000);
    assert_eq!(index_fetches(&boxes).last().unwrap(), &new.to_string());
    let at = places(&index);
    assert!(at.contains_key("PR #38 merged"));
    assert!(!at.contains_key("Photo 51"), "forgotten once gone");
    let index = index.lock().unwrap();
    let pr = index
        .records
        .values()
        .find(|r| r.subject == "PR #38 merged");
    assert_eq!(pr.unwrap().first_seen, 2_000_000);
}

#[test]
fn a_message_moved_keeps_its_key_and_its_first_sight() {
    let (account, _) = serve_boxes(boxes_in(Mode::Keywords));
    let index = Mutex::new(Index::memory());
    let mut imap = Imap::new(account);
    sync_once(&mut imap, &index);
    let key = |index: &Mutex<Index>| {
        let index = index.lock().unwrap();
        let r = index
            .records
            .values()
            .find(|r| r.subject == "Photo 51")
            .unwrap();
        (r.key.clone(), r.first_seen, r.places.clone())
    };
    let (before, seen, _) = key(&index);
    imap.relocate("INBOX", 11, "Archive", "").unwrap();
    sync_at(&mut imap, &index, 3_000_000);
    let (after, still, places) = key(&index);
    assert_eq!((after, still), (before, seen));
    assert_eq!(places.len(), 1);
    assert_eq!(places[0].folder, "Archive");
    assert_ne!(places[0].uid, 11);
    assert_eq!(index.lock().unwrap().records.len(), 3);
}

#[test]
fn a_folder_whose_uidvalidity_changed_is_read_again() {
    let (account, boxes) = serve_boxes(boxes_in(Mode::Keywords));
    let index = Mutex::new(Index::memory());
    let mut imap = Imap::new(account);
    sync_once(&mut imap, &index);
    {
        let mut fake = boxes.lock().unwrap();
        let inbox = &mut fake[0];
        inbox.validity = 8;
        for (i, l) in inbox.letters.iter_mut().enumerate() {
            l.uid = 100 + i as u32;
        }
        inbox.next = 200;
    }
    sync_once(&mut imap, &index);
    let index = index.lock().unwrap();
    assert_eq!(index.records.len(), 3);
    for r in index.records.values() {
        assert_eq!(r.places.len(), 1);
        assert_eq!((r.places[0].validity, r.places[0].uid >= 100), (8, true));
    }
}

#[test]
fn flags_changed_on_the_server_are_read_by_mod_sequence_or_again() {
    for mode in [Mode::Keywords, Mode::Folders] {
        let (account, boxes) = serve_boxes(boxes_in(mode));
        let index = Mutex::new(Index::memory());
        let mut imap = Imap::new(account);
        sync_once(&mut imap, &index);
        {
            let mut fake = boxes.lock().unwrap();
            let seq = fake.tick();
            let emmy = &mut fake[0].letters[2];
            emmy.flags.push("\\Seen".into());
            emmy.modseq = seq;
        }
        sync_at(&mut imap, &index, 1_000_100);
        let index = index.lock().unwrap();
        let emmy = index
            .records
            .values()
            .find(|r| r.address == "noether@goettingen.example")
            .unwrap();
        assert!(emmy.flags.seen, "{mode:?}");
        let said = boxes.lock().unwrap().said.join("\n");
        assert_eq!(
            said.contains("CHANGEDSINCE"),
            mode == Mode::Keywords,
            "{said}"
        );
    }
}

#[test]
fn gmail_is_read_through_all_mail_alone() {
    let (account, boxes) = serve_boxes(gmail_boxes());
    let index = Mutex::new(Index::memory());
    let mut imap = Imap::new(account);
    assert!(sync_once(&mut imap, &index).complete);
    let said = boxes.lock().unwrap().said.clone();
    let selected: Vec<&String> = said.iter().filter(|s| s.starts_with("SELECT")).collect();
    assert!(
        selected.iter().all(|s| s.contains("All Mail")),
        "{selected:?}"
    );
    let index = index.lock().unwrap();
    assert_eq!(index.records.len(), 3);
    let aims = index
        .records
        .values()
        .find(|r| r.subject == "Aims page, third pass")
        .unwrap();
    assert_eq!(aims.key.id, "gm:9021");
    assert_eq!(aims.server, ["\\Inbox", "Projects/Grant"]);
}

#[test]
fn the_demo_is_mapped_from_a_twin_that_sees_its_mail() {
    let demo = Demo::new();
    let mut twin = demo.twin().unwrap();
    let index = Mutex::new(Index::memory());
    sync_once(twin.as_mut(), &index);
    // The Inbox, Archive, Sent and Projects/Grant; not Drafts.
    assert_eq!(index.lock().unwrap().records.len(), INBOX + 20 + 4 + 4);
    let mut demo = demo;
    demo.relocate("INBOX", 1, "Trash", "").unwrap();
    sync_once(twin.as_mut(), &index);
    assert_eq!(index.lock().unwrap().records.len(), INBOX + 20 + 4 + 4 - 1);
}

#[test]
fn the_app_maps_every_account_as_it_starts() {
    let app = demo();
    assert_eq!(app.index.lock().unwrap().records.len(), INBOX + 20 + 4 + 4);
    assert!(app.mapping.is_empty(), "{}", app.mapping);
}

// ---- labels, kept on the servers ----

#[test]
fn a_label_is_a_keyword_and_back() {
    use crate::index::labels::{keyword, unkeyword};
    assert_eq!(keyword("Grant"), "Grant");
    assert_eq!(keyword("R01/aims:v2"), "R01/aims:v2");
    assert_eq!(keyword("Big Things"), "Big&ACA-Things");
    assert_eq!(keyword("Tom & Jerry"), "Tom&ACA-&-&ACA-Jerry");
    for label in [
        "Grant",
        "Big Things",
        "Tom & Jerry",
        "Entwürfe",
        "日本語のラベル",
        "a(b)c",
    ] {
        let k = keyword(label);
        assert!(
            k.chars()
                .all(|c| c.is_ascii_alphanumeric() || "._+/:-&,".contains(c)),
            "{k}"
        );
        assert_eq!(unkeyword(&k), label);
    }
    let long = "ünïcode ".repeat(20);
    let k = keyword(&long);
    assert!(k.len() <= crate::index::labels::KEYWORD, "{}", k.len());
    assert!(long.starts_with(&unkeyword(&k)));
}

#[test]
fn folders_are_labels_at_every_level() {
    use crate::index::FolderState;
    use crate::index::labels::{of_folder, server, system};
    let state = |role| FolderState {
        role: Some(role),
        delimiter: Some("/".into()),
        ..Default::default()
    };
    assert_eq!(
        of_folder("Projects/Grant", Some(&state(Role::Other))),
        ["Projects", "Grant"]
    );
    assert_eq!(of_folder("INBOX/Lab", Some(&state(Role::Other))), ["Lab"]);
    assert_eq!(
        of_folder("Entw&APw-rfe", Some(&state(Role::Other))),
        ["Entwürfe"]
    );
    assert!(of_folder("Sent Items", Some(&state(Role::Sent))).is_empty());

    let mut r = record("a", "Photo 51");
    r.places.push(index::Place {
        folder: "Projects/Grant".into(),
        validity: 7,
        uid: 3,
    });
    r.server = vec!["Lab".into(), "$Junk".into()];
    let folders = [
        ("INBOX", state(Role::Inbox)),
        ("Projects/Grant", state(Role::Other)),
    ]
    .into_iter()
    .map(|(f, s)| (("a@b.c".to_string(), f.to_string()), s))
    .collect();
    assert_eq!(server(&r, &folders), ["Projects", "Grant", "Lab", "$Junk"]);
    assert!(system(&r, &folders).inbox);
    // Gmail's labels nest as its folders do; its system labels are not.
    r.key.id = "gm:1".into();
    r.places.truncate(1);
    r.server = vec![
        "\\Inbox".into(),
        "\\Important".into(),
        "Projects/Grant".into(),
    ];
    assert_eq!(server(&r, &folders), ["Projects", "Grant"]);
    assert!(system(&r, &folders).important);
}

/// The message of `index` with `subject`: its key, labels, and whether it
/// is in the Inbox.
fn labelled(index: &Mutex<Index>, subject: &str) -> (index::Key, Vec<String>, bool) {
    let index = index.lock().unwrap();
    let r = index
        .records
        .values()
        .find(|r| r.subject == subject)
        .unwrap_or_else(|| panic!("no {subject}"));
    (
        r.key.clone(),
        index.labels(&r.key),
        index.system(&r.key).inbox,
    )
}

/// Say that the message `key` is filed under `label`, as the app does
/// before its server is asked.
fn pending(index: &Mutex<Index>, key: &index::Key, label: &str) {
    index
        .lock()
        .unwrap()
        .keep(vec![Kept::Pending {
            key: key.clone(),
            label: label.into(),
            on: true,
            at: 1_000_000,
        }])
        .unwrap();
}

#[test]
fn filing_on_a_server_of_keywords_labels_it_and_archives_it() {
    let (account, boxes) = serve_boxes(boxes_in(Mode::Keywords));
    let index = Mutex::new(Index::memory());
    let mut imap = Imap::new(account);
    sync_once(&mut imap, &index);
    let (key, labels, inbox) = labelled(&index, "Photo 51");
    assert!(labels.is_empty() && inbox);
    pending(&index, &key, "Big Things");
    let (_, labels, inbox) = labelled(&index, "Photo 51");
    assert_eq!((labels, inbox), (vec!["Big Things".to_string()], false));
    let filed = imap.file("INBOX", 11, "Big Things", "").unwrap();
    let (to, uid) = filed.moved.unwrap();
    assert_eq!(to, "Archive");
    let fake = boxes.lock().unwrap();
    let archived = fake.iter().find(|b| b.name == "Archive").unwrap();
    assert_eq!(archived.letters[0].uid, uid);
    assert!(archived.letters[0].has("Big&ACA-Things"));
    drop(fake);
    sync_at(&mut imap, &index, 1_000_100);
    assert!(index.lock().unwrap().keep.pending.is_empty(), "settled");
    let (still, labels, inbox) = labelled(&index, "Photo 51");
    assert_eq!(
        (still, labels, inbox),
        (key, vec!["Big Things".to_string()], false)
    );

    // Renamed, the keyword is changed wherever it is.
    imap.rename_label("Big Things", "Funding").unwrap();
    sync_at(&mut imap, &index, 1_000_200);
    assert_eq!(labelled(&index, "Photo 51").1, ["Funding"]);
}

#[test]
fn filing_on_gmail_gives_the_label_and_takes_the_inbox_off() {
    let (account, boxes) = serve_boxes(gmail_boxes());
    let index = Mutex::new(Index::memory());
    let mut imap = Imap::new(account);
    sync_once(&mut imap, &index);
    let (key, _, inbox) = labelled(&index, "R01 resubmission");
    assert!(inbox);
    pending(&index, &key, "Grant");
    let filed = imap.file("[Gmail]/All Mail", 22, "Grant", "").unwrap();
    assert_eq!(filed.moved, None);
    let labels = boxes.lock().unwrap()[2].letters[1].labels.clone();
    assert_eq!(labels, ["\\Important", "Grant"]);
    sync_at(&mut imap, &index, 1_000_100);
    assert!(index.lock().unwrap().keep.pending.is_empty());
    let (_, labels, inbox) = labelled(&index, "R01 resubmission");
    assert_eq!((labels, inbox), (vec!["Grant".to_string()], false));
    // A label renamed is renamed on every message it is on.
    imap.rename_label("Grant", "Funding").unwrap();
    sync_at(&mut imap, &index, 1_000_200);
    assert_eq!(labelled(&index, "R01 resubmission").1, ["Funding"]);
}

#[test]
fn filing_on_a_server_of_folders_moves_it_to_the_folder_the_label_is() {
    let (account, boxes) = server();
    let index = Mutex::new(Index::memory());
    let mut imap = Imap::new(account);
    sync_once(&mut imap, &index);
    // Grant is Projects/Grant: the folder of that name, not a new one.
    let filed = imap.file("INBOX", 11, "grant", "").unwrap();
    assert_eq!(filed.moved.as_ref().unwrap().0, "Projects/Grant");
    assert!(filed.on_server);
    sync_once(&mut imap, &index);
    assert_eq!(labelled(&index, "Photo 51").1, ["Projects", "Grant"]);
    // Out of the Inbox, a second label is letterbox's to keep.
    let (to, uid) = filed.moved.unwrap();
    let again = imap.file(&to, uid, "Lab", "").unwrap();
    assert_eq!((again.moved, again.on_server), (None, false));
    // No folder of the name: one is made, at the top.
    let new = imap.file("INBOX", 14, "Reading list", "").unwrap();
    assert_eq!(new.moved.unwrap().0, "Reading list");
    assert!(imap.label("INBOX", 12, "Lab", true).is_err());
    imap.rename_label("Grant", "Funding").unwrap();
    let names: Vec<String> = boxes
        .lock()
        .unwrap()
        .iter()
        .map(|b| b.name.clone())
        .collect();
    assert!(names.contains(&"Projects/Funding".to_string()), "{names:?}");
}

#[test]
fn the_demo_files_as_a_server_of_keywords_does() {
    let mut demo = Demo::new();
    let mut twin = demo.twin().unwrap();
    let index = Mutex::new(Index::memory());
    sync_once(twin.as_mut(), &index);
    let (key, _, inbox) = labelled(&index, "Aims page, third pass");
    assert!(inbox);
    let uid = demo
        .page("INBOX", 0, 100)
        .unwrap()
        .summaries
        .into_iter()
        .find(|s| s.subject == "Aims page, third pass")
        .unwrap()
        .uid;
    pending(&index, &key, "Grant");
    let filed = demo.file("INBOX", uid, "Grant", "").unwrap();
    assert_eq!(filed.moved.unwrap().0, "Archive");
    sync_at(twin.as_mut(), &index, 1_000_100);
    assert!(index.lock().unwrap().keep.pending.is_empty());
    let (still, labels, inbox) = labelled(&index, "Aims page, third pass");
    assert_eq!(
        (still, labels, inbox),
        (key, vec!["Grant".to_string()], false)
    );
    // The Grant folder's letters are Projects and Grant; the keyword is the
    // same label, renamed with the folder.
    assert_eq!(
        labelled(&index, "Letter of support, signed").1,
        ["Projects", "Grant"]
    );
    demo.rename_label("Grant", "Funding").unwrap();
    sync_at(twin.as_mut(), &index, 1_000_200);
    assert_eq!(labelled(&index, "Aims page, third pass").1, ["Funding"]);
    assert_eq!(
        labelled(&index, "Letter of support, signed").1,
        ["Projects", "Funding"]
    );
}

// ---- the model, on the demo's mail ----

/// The demo's mail, as the index has it.
fn demo_index() -> Index {
    let demo = Demo::new();
    let mut twin = demo.twin().unwrap();
    let index = Mutex::new(Index::memory());
    sync_once(twin.as_mut(), &index);
    index.into_inner().unwrap()
}

/// The model of `index`, read by `lens`.
fn model_of(index: &Index, lens: Lens) -> crate::model::Model {
    let given: Vec<crate::model::Given> = index
        .records
        .values()
        .map(|r| (r, index.labels(&r.key), index.system(&r.key).sent))
        .collect();
    crate::model::Model::build(&given, &index.keep.not_it, lens)
}

/// The grant's mail, by subject.
const GRANT_MAIL: [&str; 8] = [
    "R01 resubmission: internal deadline 3 November",
    "Aims page, third pass",
    "Budget justification: the sequencing line",
    "Subaward paperwork for the second site",
    "Letter of support, signed",
    "Re: Aim 2 and the spatial data",
    "Annual progress report due 15 November",
    "Biosketch format changed again",
];

fn at_subject(index: &Index, m: &crate::model::Model, subject: &str) -> usize {
    m.keys
        .iter()
        .position(|k| index.records[k].subject == subject)
        .unwrap_or_else(|| panic!("no {subject}"))
}

#[test]
fn the_grant_mail_clusters_together() {
    let index = demo_index();
    let m = model_of(&index, Lens::Both);
    let c = crate::model::cluster(&m, &[], 1);
    let aims = c.of[at_subject(&index, &m, "Aims page, third pass")];
    let together = GRANT_MAIL
        .iter()
        .filter(|s| c.of[at_subject(&index, &m, s)] == aims)
        .count();
    assert!(together >= 6, "{together} of the grant's 8 together");
    assert_eq!(c.clusters[aims].label.as_deref(), Some("Grant"));
    // The robots' mail is a cluster of its own, the grant's apart from it.
    let build = c.of[at_subject(&index, &m, "Build failed: letterbox main (clippy)")];
    let passed = c.of[at_subject(&index, &m, "Build passed: letterbox main")];
    assert_eq!(build, passed);
    assert_ne!(build, aims);
    // A cluster keeps its number from one clustering to the next.
    let before: Vec<_> = c
        .clusters
        .iter()
        .map(|cl| {
            let keys = cl.members.iter().map(|&i| m.keys[i].clone()).collect();
            (cl.id, cl.centre, keys)
        })
        .collect();
    let again = crate::model::cluster(&m, &before, 1);
    let ids =
        |c: &crate::model::Clustering| -> Vec<u32> { c.clusters.iter().map(|x| x.id).collect() };
    assert_eq!(ids(&again), ids(&c));
    assert_eq!(
        again.clusters[again.of[at_subject(&index, &m, "Aims page, third pass")]].id,
        c.clusters[aims].id
    );
}

#[test]
fn an_answer_lies_with_the_mail_it_answers_not_with_the_rest_sent() {
    let index = demo_index();
    let mira = "mira.okafor@uni.example";
    let reply = "Re: Biosketch format changed again";
    let people = model_of(&index, Lens::People);
    let r = at_subject(&index, &people, reply);
    assert!(index.system(&people.keys[r]).sent);
    let near = people.nearest(r, 3);
    let from = |i: usize| index.records[&people.keys[i]].address.as_str();
    assert!(
        near.iter().all(|&(i, _)| from(i) == mira),
        "{:?}",
        near.iter().map(|&(i, _)| from(i)).collect::<Vec<_>>()
    );
    // Read by both, it is among Mira's mail, near what it answers.
    let both = model_of(&index, Lens::Both);
    let r = at_subject(&index, &both, reply);
    let answered = at_subject(&index, &both, "Biosketch format changed again");
    let near = both.nearest(r, 2);
    assert_eq!(index.records[&both.keys[near[0].0]].address, mira);
    assert!(near.iter().any(|&(i, _)| i == answered));
    // Mail sent to different people is not drawn together for being sent.
    for m in [&people, &both] {
        let sent: Vec<usize> = (0..m.len())
            .filter(|&i| index.system(&m.keys[i]).sent)
            .collect();
        assert!(sent.len() >= 4);
        for &i in &sent {
            let (j, _) = m.nearest(i, 1)[0];
            assert!(
                !sent.contains(&j),
                "{} by {}",
                index.records[&m.keys[i]].subject,
                index.records[&m.keys[j]].subject
            );
        }
    }
}

#[test]
fn the_same_seed_lays_out_the_same_map() {
    let index = demo_index();
    let m = model_of(&index, Lens::Both);
    let a = crate::model::layout(&m, None, 1);
    let b = crate::model::layout(&m, None, 1);
    let bits = |p: &[crate::model::Point]| -> Vec<u32> {
        p.iter()
            .flat_map(|q| [q[0].to_bits(), q[1].to_bits()])
            .collect()
    };
    assert_eq!(bits(&a), bits(&b));
    assert_ne!(bits(&a), bits(&crate::model::layout(&m, None, 2)));
    // Faithful to the angles: most of each message's nearest are near it
    // on the map too, far more than chance would have it.
    let f = crate::model::faithfulness(&m, &a);
    let chance = 10.0 / (m.len() - 1) as f32;
    assert!(
        f > 2.5 * chance,
        "faithfulness {f}, against {chance} by chance"
    );
    // Begun from where it was, it moves little.
    let start: Vec<Option<crate::model::Point>> = a.iter().copied().map(Some).collect();
    let gentle = crate::model::layout(&m, Some(&start), 1);
    let fitted = crate::model::procrustes(&gentle, &a, &gentle);
    let spread: f32 = a.iter().map(|p| p[0].abs() + p[1].abs()).sum::<f32>() / a.len() as f32;
    let moved: f32 = fitted
        .iter()
        .zip(&a)
        .map(|(p, q)| (p[0] - q[0]).abs() + (p[1] - q[1]).abs())
        .sum::<f32>()
        / a.len() as f32;
    assert!(
        moved < spread,
        "moved {moved}, against a spread of {spread}"
    );
}

#[test]
fn the_map_is_the_same_on_any_number_of_threads() {
    let index = demo_index();
    let on = |threads: usize| {
        rayon::ThreadPoolBuilder::new()
            .num_threads(threads)
            .build()
            .unwrap()
            .install(|| {
                let m = model_of(&index, Lens::Both);
                let c = crate::model::cluster(&m, &[], 1);
                assert!(m.vectors.iter().all(|v| v.iter().all(|x| x.is_finite())));
                let mut at = crate::model::layout(&m, None, 1);
                at.push([m.vectors[0][0], c.of.iter().sum::<usize>() as f32]);
                at
            })
    };
    let bits = |p: &[crate::model::Point]| -> Vec<u32> {
        p.iter()
            .flat_map(|q| [q[0].to_bits(), q[1].to_bits()])
            .collect()
    };
    assert_eq!(bits(&on(1)), bits(&on(4)));
}

#[test]
fn the_nearest_found_by_tokens_are_mostly_the_nearest_of_all() {
    // Mail enough that common tokens are looked through a window only.
    let (records, _) = made_mail(4_000, 3);
    let given: Vec<crate::model::Given> = records.iter().map(|r| (r, Vec::new(), false)).collect();
    let m = crate::model::Model::build(&given, &Default::default(), Lens::Both);
    let every = crate::model::nearest_by_every_pair(&given, Lens::Both, 15, (0..4_000).step_by(20));
    let mut shared = 0;
    let mut all = 0;
    for (i, best) in every {
        shared += best
            .iter()
            .filter(|x| m.near[i].iter().any(|y| y.0 == x.0))
            .count();
        all += best.len();
    }
    let recall = shared as f32 / all as f32;
    assert!(recall > 0.9, "{recall} of the nearest found");
}

#[test]
fn a_new_message_lands_among_its_kind() {
    let mut index = demo_index();
    let m = model_of(&index, Lens::Both);
    let laid = crate::model::layout(&m, None, 1);
    // A build that failed, new: placed among the others, by its nearest.
    let indexed = index::record::Indexed {
        uid: 999,
        size: 300,
        raw: raw(
            "CI <ci@runner.example>",
            "Build failed: letterbox main (test)",
            "Content-Type: text/plain; charset=utf-8\r\nList-Id: <ci.runner.example>\r\n",
            "test map::cells failed.\r\n",
        ),
        ..Default::default()
    };
    let r = index::Record::of("demo", "INBOX", 1, &indexed, 0);
    let key = r.key.clone();
    index.cache(vec![Cached::Upsert(Box::new(r))]).unwrap();
    let m2 = model_of(&index, Lens::Both);
    let known: Vec<Option<crate::model::Point>> = m2
        .keys
        .iter()
        .map(|k| m.keys.iter().position(|x| x == k).map(|i| laid[i]))
        .collect();
    let new = m2.keys.iter().position(|k| *k == key).unwrap();
    let at = crate::model::place(&m2, new, &known);
    let mut by_place: Vec<(f32, usize)> = known
        .iter()
        .enumerate()
        .filter_map(|(i, p)| p.map(|p| ((p[0] - at[0]).powi(2) + (p[1] - at[1]).powi(2), i)))
        .collect();
    by_place.sort_by(|a, b| a.0.total_cmp(&b.0));
    let nearest = &index.records[&m2.keys[by_place[0].1]];
    assert_eq!(nearest.address, "ci@runner.example", "{}", nearest.subject);
}

#[test]
fn filing_one_grant_message_makes_the_others_suggestions() {
    let mut index = demo_index();
    let before = model_of(&index, Lens::Both);
    let r01 = before.keys[at_subject(&index, &before, GRANT_MAIL[0])].clone();
    let budget = at_subject(&index, &before, GRANT_MAIL[2]);
    index
        .keep(vec![Kept::Pending {
            key: r01.clone(),
            label: "Grant".into(),
            on: true,
            at: 0,
        }])
        .unwrap();
    let m = model_of(&index, Lens::Both);
    let not_it = index.keep.not_it.clone();
    let said: Vec<&str> = m
        .suggestions("Grant", &not_it)
        .iter()
        .map(|&(i, _)| index.records[&m.keys[i]].subject.as_str())
        .collect();
    assert!(!said.contains(&GRANT_MAIL[0]), "filed, not suggested");
    // The budget, and your answer to Mira about the biosketch; not the
    // aims page, which lies on the edge of what is near.
    for s in [GRANT_MAIL[2], "Re: Biosketch format changed again"] {
        assert!(said.contains(&s), "{s} in {said:?}");
    }
    // Your answer to Mira about the biosketch is the grant's too.
    let grant = said
        .iter()
        .take(2)
        .filter(|s| GRANT_MAIL.contains(s) || s.contains("Biosketch"))
        .count();
    assert_eq!(grant, 2, "the nearest are the grant's: {said:?}");
    let now = crate::model::degrees(&m.vectors[budget], &m.label_vectors["grant"].1);
    // Said not to be: no longer suggested, and pushed away.
    let budget_key = m.keys[budget].clone();
    index
        .keep(vec![Kept::NotIt {
            key: budget_key,
            label: "Grant".into(),
        }])
        .unwrap();
    let m = model_of(&index, Lens::Both);
    let said: Vec<&str> = m
        .suggestions("Grant", &index.keep.not_it)
        .iter()
        .map(|&(i, _)| index.records[&m.keys[i]].subject.as_str())
        .collect();
    assert!(!said.contains(&GRANT_MAIL[2]), "{said:?}");
    let pushed = crate::model::degrees(&m.vectors[budget], &m.label_vectors["grant"].1);
    assert!(pushed > now, "pushed away: {now} to {pushed}");
}

#[test]
fn two_rebuilds_are_byte_identical() {
    let build = || {
        let index = demo_index();
        let m = model_of(&index, Lens::Both);
        let c = crate::model::cluster(&m, &[], 1);
        let laid = crate::model::layout(&m, None, 1);
        let hues = m.hues();
        let mut bytes = Vec::new();
        for (i, p) in laid.iter().enumerate() {
            bytes.extend(p[0].to_le_bytes());
            bytes.extend(p[1].to_le_bytes());
            bytes.extend(hues[i].to_le_bytes());
            bytes.extend((c.of[i] as u32).to_le_bytes());
        }
        bytes
    };
    assert_eq!(build(), build());
}

#[test]
fn hues_follow_the_angle_and_procrustes_undoes_a_turn() {
    let index = demo_index();
    let m = model_of(&index, Lens::Both);
    let hues = m.hues();
    assert!(hues.iter().all(|h| (0.0..360.0).contains(h)));
    let a = at_subject(&index, &m, "Build failed: letterbox main (clippy)");
    let b = at_subject(&index, &m, "Build failed: faba (test timeout)");
    let apart = (hues[a] - hues[b]).abs();
    assert!(
        apart.min(360.0 - apart) < 60.0,
        "{} and {}",
        hues[a],
        hues[b]
    );
    // Turned, mirrored, grown and moved: laid back where it was.
    let old: Vec<crate::model::Point> = vec![[0.0, 0.0], [1.0, 0.0], [0.0, 2.0], [3.0, 1.0]];
    let new: Vec<crate::model::Point> = old
        .iter()
        .map(|p| [2.0 * p[1] + 5.0, 2.0 * p[0] - 1.0])
        .collect();
    let back = crate::model::procrustes(&new, &old, &new);
    for (p, q) in back.iter().zip(&old) {
        assert!(
            (p[0] - q[0]).abs() < 1e-4 && (p[1] - q[1]).abs() < 1e-4,
            "{p:?} {q:?}"
        );
    }
}

// ---- the map, on the screen ----

/// A click on the map at `x`, `y`: pressed and let go in the same cell.
fn map_click(app: &mut App, x: u16, y: u16) {
    let _ = app::screen(app, 120, 40);
    app.mouse(mouse(MouseEventKind::Down(MouseButton::Left), x, y));
    app.mouse(mouse(MouseEventKind::Up(MouseButton::Left), x, y));
    app.settle(Duration::from_secs(5));
}

/// The demo, its map shown.
fn demo_map() -> App {
    let mut app = demo();
    press(&mut app, &["<m-g>"]);
    app
}

#[test]
fn the_map_and_the_list_are_clicked_at_the_top_or_super_g() {
    let mut app = demo();
    let map = crate::keymap::key_named("M-g").unwrap();
    let list = crate::keymap::key_named("M-l").unwrap();
    click_at(&mut app, app::Target::Key(map));
    assert_eq!(app.map.mode, crate::map::Mode::Map);
    let screen = app::screen(&mut app, 120, 40);
    assert!(
        screen.lines().next().unwrap().contains(" List  Map "),
        "{screen}"
    );
    click_at(&mut app, app::Target::Key(list));
    assert_eq!(app.map.mode, crate::map::Mode::List);
    // ⌘G or Super+G, as a terminal that says Super says it.
    app.key(KeyEvent::new(KeyCode::Char('g'), KeyModifiers::SUPER));
    app.settle(Duration::from_secs(5));
    assert_eq!(app.map.mode, crate::map::Mode::Map);
    // Ctrl+Shift+G is no longer the map's.
    press(&mut app, &["<m-l>"]);
    app.key(KeyEvent::new(
        KeyCode::Char('G'),
        KeyModifiers::CONTROL | KeyModifiers::SHIFT,
    ));
    assert_eq!(app.map.mode, crate::map::Mode::List);
}

/// The subject of the message picked on the map.
fn picked_subject(app: &App) -> String {
    let s = app.map.snapshot.as_ref().unwrap();
    s.dots[app.map.picked().expect("one picked")]
        .subject
        .clone()
}

#[test]
fn the_map_lays_out_every_message_with_its_clusters() {
    let mut app = demo_map();
    assert_eq!(app.map.mode, crate::map::Mode::Map);
    let s = app.map.snapshot.clone().expect("drawn");
    assert_eq!(s.dots.len(), INBOX + 20 + 4 + 4);
    assert!(s.at.is_some(), "laid out");
    // The layout is kept, to be the same next time.
    assert_eq!(
        app.index.lock().unwrap().keep.layouts[&Lens::Both].len(),
        s.dots.len()
    );
    let screen = app::screen(&mut app, 150, 45);
    assert!(screen.contains("Map · words and people"), "{screen}");
    assert!(screen.contains("◆  Grant"), "{screen}");
    assert!(
        screen.contains(" MAP") && screen.contains("Every cluster"),
        "{screen}"
    );
    assert!(screen.contains("% faithful"), "{screen}");
    let wide = app::screen(&mut app, 220, 45);
    assert!(wide.contains("colour is what it is about"), "{wide}");
    // The newest new message is picked, named beside it, and read beside
    // the map once the cursor rests.
    assert_eq!(
        picked_subject(&app),
        "Notes on the engine, and a table of Bernoulli numbers"
    );
    assert!(
        screen.contains("Ada Lovelace — Notes on the engine"),
        "{screen}"
    );
    let foot: Vec<&str> = screen.lines().rev().take(2).collect();
    let foot = foot.join(" ");
    assert!(
        foot.contains("M-l list") && foot.contains("m move to"),
        "{foot}"
    );
}

#[test]
fn the_cursor_moves_to_the_nearest_message_that_way_and_reads_it() {
    let mut app = demo_map();
    let _ = app::screen(&mut app, 150, 45);
    let first = picked_subject(&app);
    let (x0, _) = app.map.cells[&app.map.picked().unwrap()];
    press(&mut app, &["l"]);
    let (x1, _) = app.map.cells[&app.map.picked().unwrap()];
    assert!(x1 >= x0, "right: {x0} to {x1}");
    assert_ne!(picked_subject(&app), first);
    // Read beside the map, once the cursor rests.
    let read = app.message().map(|m| m.subject.clone());
    assert_eq!(read, Some(picked_subject(&app)));
    press(&mut app, &["h"]);
    let (x2, _) = app.map.cells[&app.map.picked().unwrap()];
    assert!(x2 <= x1, "left: {x1} to {x2}");
    let screen = app::screen(&mut app, 150, 45);
    assert!(
        screen.contains("Here") && screen.contains("Near it, by angle"),
        "{screen}"
    );
}

#[test]
fn the_map_is_narrowed_to_a_cluster_and_listed() {
    let mut app = demo_map();
    press(&mut app, &["]"]);
    let crate::map::Focused::Cluster(id) = app.map.focused else {
        panic!("not narrowed: {:?}", app.map.focused);
    };
    let group = app
        .map
        .snapshot
        .as_ref()
        .unwrap()
        .group(id)
        .unwrap()
        .clone();
    assert!(group.members.contains(&app.map.picked().unwrap()));
    press(&mut app, &["<esc>"]);
    assert_eq!(app.map.focused, crate::map::Focused::All);
    // The Grant cluster in the sidebar: in the map it narrows it, in the
    // list it lists it.
    let grant = app
        .rows
        .iter()
        .position(|r| match r {
            app::Row::Cluster(id) => {
                app.map
                    .snapshot
                    .as_ref()
                    .unwrap()
                    .group(*id)
                    .unwrap()
                    .label
                    .as_deref()
                    == Some("Grant")
            }
            _ => false,
        })
        .expect("Grant in the sidebar");
    click_at(&mut app, app::Target::Row(grant));
    assert!(matches!(app.map.focused, crate::map::Focused::Cluster(_)));
    press(&mut app, &["<m-l>"]);
    assert_eq!(app.map.mode, crate::map::Mode::List);
    assert_eq!(app.folder_name(), "Grant");
    let subjects: Vec<&str> = app.shown.iter().map(|m| m.subject.as_str()).collect();
    assert!(subjects.contains(&"Aims page, third pass"), "{subjects:?}");
    // Read from the list, it is read on its own account and folder.
    let aims = app
        .shown
        .iter()
        .position(|m| m.subject == "Aims page, third pass")
        .unwrap();
    app.list.select(Some(aims));
    press(&mut app, &["<enter>"]);
    assert_eq!(app.message().unwrap().subject, "Aims page, third pass");
    press(&mut app, &["<m-g>"]);
    assert_eq!(app.map.mode, crate::map::Mode::Map);
}

#[test]
fn the_lens_the_time_the_colour_and_read_mail_are_turned() {
    let mut app = demo_map();
    let all = app.map.shown(jiff::Timestamp::now().as_second()).len();
    // From the view menu, which stays open to change more.
    press(&mut app, &[",", "w"]);
    assert!(matches!(app.dialog, app::Dialog::View));
    assert_eq!(app.map.lens, Lens::Words);
    assert_eq!(app.map.snapshot.as_ref().unwrap().lens, Lens::Words);
    assert!(
        app.index
            .lock()
            .unwrap()
            .keep
            .layouts
            .contains_key(&Lens::Words)
    );
    press(&mut app, &["w", "w"]);
    assert_eq!(app.map.lens, Lens::Both);
    press(&mut app, &["t"]);
    let now = jiff::Timestamp::now().as_second();
    let week = app.map.shown(now).len();
    assert!(week < all, "{week} of {all}");
    press(&mut app, &["t", "t"]);
    press(&mut app, &["r"]);
    let unread = app.map.shown(now).len();
    assert_eq!(unread, UNREAD as usize);
    press(&mut app, &["r", "c"]);
    assert_eq!(app.map.colour, crate::map::Colour::Cursor);
    let screen = app::screen(&mut app, 150, 45);
    assert!(screen.contains("how near the one picked"), "{screen}");
    press(&mut app, &["c", "c", "n"]);
    assert!(app.map.new_only);
    press(&mut app, &["s"]);
    assert!(app.map.hide_sent);
    // Any other key closes it; l lays the map out again.
    press(&mut app, &["<esc>"]);
    assert!(matches!(app.dialog, app::Dialog::Closed));
    press(&mut app, &[",", "l"]);
    assert!(matches!(app.dialog, app::Dialog::Closed));
    assert!(app.map.preview.is_some(), "{}", app.status);
}

#[test]
fn a_click_on_the_map_picks_and_a_click_on_an_anchor_narrows() {
    let mut app = demo_map();
    let _ = app::screen(&mut app, 120, 40);
    // A cell of several messages: each click picks the next of them.
    let (&(x, y), members) = app
        .map
        .drawn
        .iter()
        .find(|(_, m)| m.len() > 1)
        .or_else(|| app.map.drawn.iter().next())
        .map(|(c, m)| (c, m.clone()))
        .unwrap();
    map_click(&mut app, x, y);
    assert_eq!(app.map.picked(), Some(members[0]));
    if members.len() > 1 {
        map_click(&mut app, x, y);
        assert_eq!(app.map.picked(), Some(members[1]));
    }
    let _ = app::screen(&mut app, 120, 40);
    let &(_, id) = app
        .targets
        .iter()
        .filter_map(|(r, t)| match t {
            app::Target::Anchor(id) => Some((r, *id)),
            _ => None,
        })
        .collect::<Vec<_>>()
        .first()
        .unwrap();
    click_at(&mut app, app::Target::Anchor(id));
    assert_eq!(app.map.focused, crate::map::Focused::Cluster(id));
}

#[test]
fn the_map_fits_any_screen_light_or_dark() {
    use crate::theme::{Name, Theme};
    for name in [Name::Light, Name::Dark] {
        for width in [16, 33, 69, 70, 119, 120, 160] {
            for height in [6, 9, 24, 48] {
                let mut app = demo_map();
                app.theme = Theme::of(name);
                let screen = app::screen(&mut app, width, height);
                assert_eq!(screen.lines().count(), height as usize);
                let _ = app::screen_svg(&mut app, width, height);
            }
        }
    }
    let mut app = demo_map();
    let narrow = app::screen(&mut app, 60, 24);
    assert!(
        narrow.contains("Map ·") && !narrow.contains("DEMO"),
        "{narrow}"
    );
}

#[test]
fn a_message_in_two_accounts_is_two_dots_marked_twins() {
    let mut app = two_accounts();
    press(&mut app, &["<m-g>"]);
    let s = app.map.snapshot.clone().unwrap();
    assert_eq!(s.dots.len(), 2 * (INBOX + 20 + 4 + 4) + 1);
    let photo: Vec<&crate::map::Dot> = s.dots.iter().filter(|d| d.subject == "Photo 51").collect();
    assert_eq!(photo.len(), 2);
    assert!(photo.iter().all(|d| d.twin));
    assert!(
        !s.dots
            .iter()
            .find(|d| d.subject == "Newest of all")
            .unwrap()
            .twin
    );
}

#[test]
fn the_map_says_why_it_is_not_there() {
    let mut app = App::with_index(
        vec![("Demo".to_string(), Box::new(Demo::new()) as Box<dyn Store>)],
        PathBuf::from("/nowhere/accounts.toml"),
        Err("the map's key would be kept in the vault, which is locked".to_string()),
    );
    app.ledger = scratch_ledger();
    app.settle(Duration::from_secs(5));
    press(&mut app, &["<m-g>"]);
    assert!(
        app.status.contains("vault, which is locked"),
        "{}",
        app.status
    );
    assert_eq!(app.map.mode, crate::map::Mode::List);
}

// ---- regions of the map ----

/// The messages of the demo's map, by subject, and whether each is read.
fn map_seen(app: &App) -> std::collections::BTreeMap<String, bool> {
    let s = app.map.snapshot.as_ref().unwrap();
    s.dots
        .iter()
        .map(|d| (d.subject.clone(), d.flags.seen))
        .collect()
}

#[test]
fn v_marks_the_cluster_and_u_marks_it_all_read() {
    let mut app = demo_map();
    let s = app.map.snapshot.clone().unwrap();
    // The cluster of the Grant's mail, with unread mail in it.
    let picked = s
        .dots
        .iter()
        .position(|d| d.subject.starts_with("R01"))
        .unwrap();
    app.map.cursor = Some(s.dots[picked].key.clone());
    press(&mut app, &["v"]);
    let region = app.map.region.clone().expect("marked");
    let cluster = s.group_of(picked).unwrap();
    assert_eq!(region.keys.len(), cluster.members.len());
    assert!(app.status.contains("marked"), "{}", app.status);
    let screen = app::screen(&mut app, 150, 45);
    assert!(screen.contains('┌') && screen.contains('│'), "{screen}");
    press(&mut app, &["u"]);
    let seen = map_seen(&app);
    for &i in &cluster.members {
        assert!(seen[&s.dots[i].subject], "{} read", s.dots[i].subject);
    }
    // On the server too.
    let r01 = app
        .shown
        .iter()
        .find(|m| m.subject.starts_with("R01"))
        .unwrap();
    assert!(r01.flags.seen);
    press(&mut app, &["u"]);
    assert!(
        !map_seen(&app)[&s.dots[picked].subject],
        "and all unread again"
    );
    press(&mut app, &["v"]);
    assert!(app.map.region.is_none());
}

#[test]
fn a_box_dragged_on_the_map_marks_what_is_in_it() {
    let mut app = demo_map();
    let _ = app::screen(&mut app, 120, 40);
    let (&first, _) = app.map.drawn.iter().next().unwrap();
    let (&last, _) = app.map.drawn.iter().next_back().unwrap();
    let ((x0, y0), (x1, y1)) = (first, last);
    let (left, right) = (x0.min(x1), x0.max(x1));
    let (top, bottom) = (y0.min(y1), y0.max(y1));
    app.mouse(mouse(MouseEventKind::Down(MouseButton::Left), left, top));
    app.mouse(mouse(
        MouseEventKind::Drag(MouseButton::Left),
        right,
        bottom,
    ));
    let screen = app::screen(&mut app, 120, 40);
    assert!(screen.contains('┌'), "drawn as it goes: {screen}");
    app.mouse(mouse(MouseEventKind::Up(MouseButton::Left), right, bottom));
    let inside = app
        .map
        .cells
        .values()
        .filter(|&&(x, y)| (left..=right).contains(&x) && (top..=bottom).contains(&y))
        .count();
    let region = app.map.region.clone().expect("marked");
    assert_eq!(region.keys.len(), inside);
    assert!(inside >= 2);
}

#[test]
fn what_is_marked_is_trashed_or_archived_at_once() {
    let mut app = demo_map();
    let s = app.map.snapshot.clone().unwrap();
    let builds: Vec<crate::index::Key> = s
        .dots
        .iter()
        .filter(|d| d.address == "ci@runner.example")
        .map(|d| d.key.clone())
        .collect();
    assert_eq!(builds.len(), 3);
    app.map.region = Some(crate::map::Region {
        keys: builds,
        boxed: None,
    });
    press(&mut app, &["d"]);
    assert!(app.map.region.is_none());
    let s = app.map.snapshot.clone().unwrap();
    assert!(
        s.dots
            .iter()
            .filter(|d| d.address == "ci@runner.example")
            .all(|d| !d.system.inbox),
        "{}",
        app.status
    );
    // Archived: out of the Inbox, into Archive.
    let hotel: Vec<crate::index::Key> = s
        .dots
        .iter()
        .filter(|d| d.address == "stay@hotel.example" || d.address == "travel@uni.example")
        .map(|d| d.key.clone())
        .collect();
    app.map.region = Some(crate::map::Region {
        keys: hotel,
        boxed: None,
    });
    press(&mut app, &["e"]);
    let s = app.map.snapshot.clone().unwrap();
    for d in s
        .dots
        .iter()
        .filter(|d| d.address == "stay@hotel.example" || d.address == "travel@uni.example")
    {
        assert!(!d.system.inbox, "{}", d.subject);
        assert!(
            d.places.iter().any(|p| p.folder == "Archive"),
            "{}",
            d.subject
        );
    }
    // A click, pressed and let go in one cell, is no box.
    let _ = app::screen(&mut app, 120, 40);
    let (&(x, y), _) = app.map.drawn.iter().next().unwrap();
    map_click(&mut app, x, y);
    assert!(app.map.region.is_none());
}

// ---- filing, from the map ----

/// Pick the message of the map with `subject`.
fn pick_subject(app: &mut App, subject: &str) {
    let s = app.map.snapshot.clone().unwrap();
    let i = s
        .dots
        .iter()
        .position(|d| d.subject == subject)
        .unwrap_or_else(|| panic!("no {subject}"));
    app.map.cursor = Some(s.dots[i].key.clone());
}

#[test]
fn keys_mean_the_same_on_the_map_as_in_the_list() {
    let mut app = demo_map();
    // r answers the message picked on the map, not the list's.
    pick_subject(&mut app, "Aims page, third pass");
    press(&mut app, &["r"]);
    let text = app.compose.as_ref().expect("written").buffer.text();
    assert!(
        text.contains("Subject: Re: Aims page, third pass"),
        "{text}"
    );
    press(&mut app, &["<c-c>", "<c-k>"]);
    // e archives it, as it archives in the list.
    press(&mut app, &["e"]);
    let (_, inbox) = dot_labels(&app, "Aims page, third pass");
    assert!(!inbox, "{}", app.status);
    // ⌘L, or Super+L, back to the list.
    app.key(KeyEvent::new(KeyCode::Char('l'), KeyModifiers::SUPER));
    assert_eq!(app.map.mode, crate::map::Mode::List);
    let first = app.shown[0].summary.subject.clone();
    press(&mut app, &["e"]);
    assert_eq!(app.status, "archived, out of the Inbox");
    assert!(app.shown.iter().all(|m| m.summary.subject != first));
}

#[test]
fn space_marks_in_the_picker_after_typing_too() {
    let dir = tempfile::tempdir().unwrap();
    for name in ["one draft.txt", "two draft.txt", "notes.txt"] {
        std::fs::write(dir.path().join(name), b"x").unwrap();
    }
    let mut p = crate::pick::Picker::open(dir.path().to_path_buf());
    for c in "draft".chars() {
        p.key(KeyEvent::new(KeyCode::Char(c), KeyModifiers::NONE));
    }
    assert_eq!(p.shown().len(), 3, "..: and the two drafts");
    p.key(KeyEvent::new(KeyCode::Char(' '), KeyModifiers::NONE));
    p.key(KeyEvent::new(KeyCode::Char(' '), KeyModifiers::NONE));
    assert_eq!(p.filter, "draft");
    let crate::pick::Picked::Take(files) = p.key(KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE))
    else {
        panic!("none taken");
    };
    let names: Vec<_> = files
        .iter()
        .map(|f| f.file_name().unwrap().to_owned())
        .collect();
    assert_eq!(names, ["one draft.txt", "two draft.txt"]);
}

/// The labels of the map's message with `subject`, and whether it is in
/// the Inbox.
fn dot_labels(app: &App, subject: &str) -> (Vec<String>, bool) {
    let s = app.map.snapshot.as_ref().unwrap();
    let d = s.dots.iter().find(|d| d.subject == subject).unwrap();
    (d.labels.clone(), d.system.inbox)
}

#[test]
fn m_moves_a_message_onto_a_cluster_and_files_it_under_its_label() {
    let mut app = demo_map();
    pick_subject(&mut app, "Aims page, third pass");
    press(&mut app, &["m"]);
    assert!(app.map.moving.is_some());
    assert!(app.status.contains("moving it"), "{}", app.status);
    // Onto the Grant's cluster, by a message of it, and ⏎.
    pick_subject(&mut app, "Letter of support, signed");
    press(&mut app, &["<enter>"]);
    assert!(app.map.moving.is_none());
    assert!(app.status.contains("under Grant"), "{}", app.status);
    let (labels, inbox) = dot_labels(&app, "Aims page, third pass");
    assert!(labels.contains(&"Grant".to_string()), "{labels:?}");
    assert!(!inbox, "out of the Inbox");
    // The server has it, the next pass says, and it is pending no more.
    assert!(app.index.lock().unwrap().keep.pending.is_empty());
    let s = app.map.snapshot.clone().unwrap();
    let aims = s
        .dots
        .iter()
        .find(|d| d.subject == "Aims page, third pass")
        .unwrap();
    assert!(aims.places.iter().any(|p| p.folder == "Archive"));
    // Its like are suggested, beside it.
    press(&mut app, &["<enter>"]);
    let screen = app::screen(&mut app, 150, 60);
    assert!(screen.contains("Would go to Grant too"), "{screen}");
}

#[test]
fn n_files_what_is_moving_under_a_new_label_named_by_its_commonest_word() {
    let mut app = demo_map();
    let s = app.map.snapshot.clone().unwrap();
    let builds: Vec<crate::index::Key> = s
        .dots
        .iter()
        .filter(|d| d.address == "ci@runner.example")
        .map(|d| d.key.clone())
        .collect();
    app.map.region = Some(crate::map::Region {
        keys: builds,
        boxed: None,
    });
    press(&mut app, &["m", "N"]);
    let app::Dialog::Label(typed) = &app.dialog else {
        panic!("not asked: {}", app.status);
    };
    assert_eq!(typed, "Build");
    press(&mut app, &["s", "<enter>"]);
    assert!(
        app.status.contains("3 filed under Builds"),
        "{}",
        app.status
    );
    for subject in [
        "Build failed: letterbox main (clippy)",
        "Build passed: letterbox main",
        "Build failed: faba (test timeout)",
    ] {
        assert_eq!(dot_labels(&app, subject).0, ["Builds"]);
    }
    // A cluster that is a label now, its anchor said so.
    let s = app.map.snapshot.clone().unwrap();
    assert!(
        s.groups
            .iter()
            .any(|g| g.label.as_deref() == Some("Builds"))
    );
}

#[test]
fn filing_suggests_its_like_and_y_and_minus_take_them_or_not() {
    let mut app = demo_map();
    pick_subject(&mut app, "R01 resubmission: internal deadline 3 November");
    press(&mut app, &["m"]);
    // Onto the Grant's anchor, by a click.
    let _ = app::screen(&mut app, 150, 45);
    let s = app.map.snapshot.clone().unwrap();
    let grant = s
        .groups
        .iter()
        .find(|g| g.label.as_deref() == Some("Grant"))
        .unwrap()
        .id;
    click_at(&mut app, app::Target::Anchor(grant));
    assert!(app.status.contains("under Grant"), "{}", app.status);
    let suggested: Vec<String> = {
        let s = app.map.snapshot.clone().unwrap();
        app.suggestions()
            .iter()
            .map(|&(i, _)| s.dots[i].subject.clone())
            .collect()
    };
    assert!(
        suggested.contains(&"Budget justification: the sequencing line".to_string()),
        "{suggested:?}"
    );
    let first = suggested[0].clone();
    press(&mut app, &["y"]);
    assert!(dot_labels(&app, &first).0.contains(&"Grant".to_string()));
    let next = {
        let s = app.map.snapshot.clone().unwrap();
        s.dots[app.suggestions()[0].0].subject.clone()
    };
    press(&mut app, &["-"]);
    assert!(app.status.contains("suggested no more"), "{}", app.status);
    let s = app.map.snapshot.clone().unwrap();
    assert!(
        app.suggestions()
            .iter()
            .all(|&(i, _)| s.dots[i].subject != next)
    );
    assert_eq!(app.index.lock().unwrap().keep.not_it.len(), 1);
    press(&mut app, &["<esc>"]);
    assert!(app.map.suggesting.is_none());
}

#[test]
fn r_renames_the_label_picked_on_its_server() {
    let mut app = demo_map();
    pick_subject(&mut app, "Letter of support, signed");
    press(&mut app, &["R"]);
    let app::Dialog::RenameLabel(old, typed) = &app.dialog else {
        panic!("not asked: {}", app.status);
    };
    assert_eq!((old.as_str(), typed.as_str()), ("Grant", "Grant"));
    press(&mut app, &["<c-u>", "Funding", "<enter>"]);
    assert!(
        app.status.contains("renamed Grant to Funding"),
        "{}",
        app.status
    );
    let (labels, _) = dot_labels(&app, "Letter of support, signed");
    assert_eq!(labels, ["Projects", "Funding"]);
    // Outside the map, R renames an account, as it did.
    press(&mut app, &["<m-l>", "R"]);
    assert!(matches!(app.dialog, app::Dialog::Rename(0, _)));
}

// ---- laying the map out again ----

#[test]
fn m_r_lays_the_map_out_again_to_keep_or_drop() {
    let mut app = demo_map();
    let kept = |app: &App| app.index.lock().unwrap().keep.layouts[&Lens::Both].clone();
    let before = kept(&app);
    press(&mut app, &["<m-r>"]);
    assert!(app.map.preview.is_some(), "{}", app.status);
    assert!(app.status.contains("esc drops it"), "{}", app.status);
    let screen = app::screen(&mut app, 150, 45);
    assert!(screen.contains("laid out again"), "{screen}");
    // Dropped, it is as it was.
    press(&mut app, &["<esc>"]);
    assert!(app.map.preview.is_none());
    assert_eq!(kept(&app), before);
    // Kept, it is the map from now on.
    press(&mut app, &["<m-r>", "<enter>"]);
    assert!(app.map.preview.is_none());
    assert!(app.status.contains("kept"), "{}", app.status);
    let after = kept(&app);
    assert_eq!(after.len(), before.len());
    assert_ne!(after, before);
    let s = app.map.snapshot.clone().unwrap();
    let at = s.at.as_ref().unwrap();
    let k = &s.model.keys[0];
    assert_eq!((at[0][0], at[0][1]), after[k]);
}

#[test]
fn faithfulness_below_seventy_percent_suggests_m_r() {
    let mut app = demo_map();
    let faith = app.map.snapshot.as_ref().unwrap().faithfulness.unwrap();
    let screen = app::screen(&mut app, 160, 45);
    let said = format!("{:.0}% faithful", faith * 100.0);
    assert!(screen.contains(&said), "{said} in {screen}");
    assert_eq!(
        screen.contains("M-r lays it out again"),
        faith < 0.7,
        "{screen}"
    );
}

/// The layouts `index` keeps, as their bits, by lens.
fn layout_bits(index: &Index) -> Vec<(Lens, Vec<u32>)> {
    index
        .keep
        .layouts
        .iter()
        .map(|(lens, at)| {
            let bits = at
                .values()
                .flat_map(|&(x, y)| [x.to_bits(), y.to_bits()])
                .collect();
            (*lens, bits)
        })
        .collect()
}

#[test]
fn a_rebuild_lays_out_the_same_map_from_the_same_mail_and_keeps_what_letterbox_knows() {
    let dir = tempfile::tempdir().unwrap();
    let id = identity();
    let mut once = demo_index();
    let said = crate::map::rebuild(&mut once).unwrap();
    assert!(said.contains("laid out 61 messages"), "{said}");
    assert_eq!(once.keep.layouts.len(), 3, "a layout each lens");
    let mut twice = demo_index();
    crate::map::rebuild(&mut twice).unwrap();
    assert_eq!(layout_bits(&once), layout_bits(&twice));

    // From the servers: what they said is let go and read again; a label
    // letterbox keeps, and a "not it", stay.
    {
        let mut index = Index::open_with(dir.path(), &id);
        let demo = Demo::new();
        let mut twin = demo.twin().unwrap();
        let shared = Mutex::new(index);
        sync_once(twin.as_mut(), &shared);
        index = shared.into_inner().unwrap();
        let key = index.records.keys().next().unwrap().clone();
        index
            .keep(vec![
                Kept::Label {
                    key: key.clone(),
                    label: "Lab".into(),
                    on: true,
                },
                Kept::NotIt {
                    key,
                    label: "Grant".into(),
                },
            ])
            .unwrap();
        index.forget_cache().unwrap();
        assert!(index.records.is_empty());
        let shared = Mutex::new(index);
        sync_once(twin.as_mut(), &shared);
        let mut index = shared.into_inner().unwrap();
        crate::map::rebuild(&mut index).unwrap();
        assert_eq!(index.records.len(), INBOX + 20 + 4 + 4);
    }
    let index = Index::open_with(dir.path(), &id);
    assert_eq!(index.records.len(), INBOX + 20 + 4 + 4);
    assert_eq!(index.keep.labels.len(), 1);
    assert_eq!(index.keep.not_it.len(), 1);
    assert_eq!(index.keep.layouts.len(), 3);
    assert!(
        index
            .keep
            .layouts
            .values()
            .all(|at| at.len() == INBOX + 20 + 4 + 4)
    );
}

// ---- the map's model on mail made up of known topics ----

/// Numbers that look random, for mail made up.
struct Dice(u64);

impl Dice {
    fn next(&mut self) -> u64 {
        self.0 = self.0.wrapping_add(0x9e37_79b9_7f4a_7c15);
        let mut z = self.0;
        z = (z ^ (z >> 30)).wrapping_mul(0xbf58_476d_1ce4_e5b9);
        z = (z ^ (z >> 27)).wrapping_mul(0x94d0_49bb_1331_11eb);
        z ^ (z >> 31)
    }

    fn unit(&mut self) -> f64 {
        (self.next() >> 11) as f64 / (1u64 << 53) as f64
    }

    fn below(&mut self, n: usize) -> usize {
        ((self.unit() * n as f64) as usize).min(n - 1)
    }

    /// One of `cumulative`'s places, as likely as its share of the last.
    fn pick(&mut self, cumulative: &[f64]) -> usize {
        let at = self.unit() * cumulative[cumulative.len() - 1];
        cumulative
            .partition_point(|&c| c < at)
            .min(cumulative.len() - 1)
    }
}

/// A made-up word, the `i`th, of letters but `s`.
fn made_word(mut i: usize) -> String {
    const LETTERS: &[u8] = b"abcdefghijklmnopqrtuvwxyz";
    let mut w = String::from("q");
    loop {
        w.push(LETTERS[i % LETTERS.len()] as char);
        i /= LETTERS.len();
        if i == 0 {
            return w;
        }
    }
}

fn zipf(n: usize, shift: f64) -> Vec<f64> {
    let mut total = 0.0;
    (0..n)
        .map(|r| {
            total += 1.0 / (r as f64 + shift);
            total
        })
        .collect()
}

/// `n` messages made up of topics, as records, and each one's topic. A
/// topic has its people, some shared with others, a list now and then,
/// and words of its own; each message mixes a topic's words with words
/// any message might have, the commonest most often; a tenth are sent.
fn made_mail(n: usize, seed: u64) -> (Vec<index::Record>, Vec<usize>) {
    let mut dice = Dice(seed);
    let words = 30_000;
    let any = zipf(words, 10.0);
    let topics = (n / 100).max(20);
    let people = topics * 2;
    let address = |p: usize| format!("p{p}@d{}.example", p % (people / 3 + 1));
    struct Topic {
        people: Vec<usize>,
        list: Option<String>,
        words: Vec<usize>,
    }
    let all: Vec<Topic> = (0..topics)
        .map(|t| {
            let list = (dice.unit() < 0.25).then(|| format!("list{t}.example"));
            let many = if list.is_some() { 1 } else { 1 + dice.below(5) };
            Topic {
                people: (0..many).map(|_| dice.below(people)).collect(),
                list,
                words: (0..30).map(|_| 200 + dice.below(words - 200)).collect(),
            }
        })
        .collect();
    let sizes = zipf(topics, 3.0);
    let mut records = Vec::with_capacity(n);
    let mut truth = Vec::with_capacity(n);
    for i in 0..n {
        let t = dice.pick(&sizes);
        let topic = &all[t];
        let word = |share: f64, dice: &mut Dice| {
            if dice.unit() < share {
                made_word(topic.words[dice.below(topic.words.len())])
            } else {
                made_word(dice.pick(&any))
            }
        };
        let subject: Vec<String> = (0..4).map(|_| word(0.6, &mut dice)).collect();
        let preview: Vec<String> = (0..20).map(|_| word(0.3, &mut dice)).collect();
        let person = address(topic.people[dice.below(topic.people.len())]);
        let sent = topic.list.is_none() && dice.unit() < 0.1;
        let mut to = vec!["you@home.example".to_string()];
        if dice.unit() < 0.3 {
            to.push(address(topic.people[dice.below(topic.people.len())]));
        }
        let (address, to) = if sent {
            ("you@home.example".to_string(), vec![person])
        } else {
            (person, to)
        };
        records.push(index::Record {
            key: index::Key {
                account: "made".into(),
                id: format!("id:{i}"),
            },
            from: address.clone(),
            address,
            to,
            list: topic.list.clone().unwrap_or_default(),
            subject: subject.join(" "),
            preview: preview.join(" "),
            date: Some(i as i64),
            ..Default::default()
        });
        truth.push(t);
    }
    (records, truth)
}

/// How the model does on made-up mail of `n` messages: how long each
/// step takes, and how many of each message's ten nearest share its topic.
fn measure(n: usize) {
    use crate::model;
    use std::collections::BTreeMap;
    use std::time::Instant;
    let (records, truth) = made_mail(n, 7);
    let given: Vec<model::Given> = records
        .iter()
        .map(|r| (r, Vec::new(), r.address == "you@home.example"))
        .collect();
    let t = Instant::now();
    let m = model::Model::build(&given, &Default::default(), Lens::Both);
    let build = t.elapsed();
    let t = Instant::now();
    let c = model::cluster(&m, &[], 1);
    let clustering = t.elapsed();
    let t = Instant::now();
    let at = model::layout(&m, None, 1);
    let layout = t.elapsed();
    let f = model::faithfulness(&m, &at);
    let sample: Vec<usize> = (0..n).step_by((n / 500).max(1)).collect();
    let share = |near: &dyn Fn(usize) -> Vec<usize>| {
        let mut same = 0;
        let mut all = 0;
        for &i in &sample {
            for j in near(i) {
                same += usize::from(truth[j] == truth[i]);
                all += 1;
            }
        }
        same as f32 / all.max(1) as f32
    };
    // Of each message's ten nearest, how many share its topic: by tokens,
    // every pair compared; as found; by the vectors; on the map.
    let every: BTreeMap<usize, Vec<usize>> =
        model::nearest_by_every_pair(&given, Lens::Both, 10, sample.iter().copied())
            .into_iter()
            .map(|(i, near)| (i, near.into_iter().map(|x| x.0).collect()))
            .collect();
    let ceiling = share(&|i| every[&i].clone());
    let found = share(&|i| m.near[i].iter().take(10).map(|x| x.0).collect());
    let vectors = share(&|i| m.nearest(i, 10).into_iter().map(|x| x.0).collect());
    let on_map = share(&|i| {
        let mut all: Vec<(usize, f32)> = (0..n)
            .filter(|&j| j != i)
            .map(|j| {
                (
                    j,
                    (at[i][0] - at[j][0]).powi(2) + (at[i][1] - at[j][1]).powi(2),
                )
            })
            .collect();
        all.sort_by(|a, b| a.1.total_cmp(&b.1).then(a.0.cmp(&b.0)));
        all.into_iter().take(10).map(|x| x.0).collect()
    });
    // Clusters: how many messages are in the cluster most of their topic is.
    let mut by_topic: BTreeMap<usize, BTreeMap<usize, usize>> = BTreeMap::new();
    for (t, &of) in truth.iter().zip(&c.of) {
        *by_topic.entry(*t).or_default().entry(of).or_default() += 1;
    }
    let pure: usize = by_topic.values().map(|m| m.values().max().unwrap()).sum();
    eprintln!(
        "n {n:>6}: build {build:>6.0?} cluster {clustering:>6.0?} layout {layout:>6.0?} | \
         topic@10 every pair {ceiling:.2} found {found:.2} vectors {vectors:.2} map {on_map:.2} | \
         kept together {:.2} | faithful {:.0}%",
        pure as f32 / n as f32,
        100.0 * f
    );
}

/// How long the map takes to follow the index, as the app has it do, on
/// `n` made-up messages: the first snapshot, then one after a new message
/// comes, and one after a message is read.
fn measure_following(n: usize) {
    use std::time::Instant;
    let (records, _) = made_mail(n + 1, 7);
    let mut index = Index::memory();
    let mut records = records.into_iter();
    let last = records.next_back().unwrap();
    index
        .cache(records.map(|r| Cached::Upsert(Box::new(r))).collect())
        .unwrap();
    let step = |index: &Index, last: Option<&mut crate::map::Snapshot>| {
        let t = Instant::now();
        let (last, corpus) = match last {
            Some(s) => {
                let corpus = s.corpus.take();
                (Some(&*s), corpus)
            }
            None => (None, None),
        };
        let input = crate::map::input(index, Lens::Both, last, corpus, true);
        let taken = t.elapsed();
        let t = Instant::now();
        let s = crate::map::compute(input);
        (s, taken, t.elapsed())
    };
    let (mut first, taken, made) = step(&index, None);
    eprintln!("n {n:>6}: first: taken {taken:>6.0?} made {made:>6.0?}");
    // Kept, as the app keeps the layout made.
    let at = first.fresh.clone().unwrap();
    index
        .keep(vec![Kept::Layout {
            lens: Lens::Both,
            at,
        }])
        .unwrap();
    let (mut s, taken, made) = step(&index, Some(&mut first));
    eprintln!("          the layout kept: taken {taken:>6.0?} made {made:>6.0?}");
    let key = last.key.clone();
    index.cache(vec![Cached::Upsert(Box::new(last))]).unwrap();
    let (mut s2, taken, made) = step(&index, Some(&mut s));
    eprintln!("          a new message: taken {taken:>6.0?} made {made:>6.0?}");
    let flags = crate::mail::Flags {
        seen: true,
        ..Default::default()
    };
    index.cache(vec![Cached::Flags { key, flags }]).unwrap();
    let (_, taken, made) = step(&index, Some(&mut s2));
    eprintln!("          a message read: taken {taken:>6.0?} made {made:>6.0?}");
}

#[test]
#[ignore = "measures; run with --release --ignored --nocapture"]
fn how_long_the_map_takes_to_follow_the_index() {
    for n in [10_000, 50_000] {
        measure_following(n);
    }
}

#[test]
#[ignore = "measures; run with --release --ignored --nocapture"]
fn how_the_model_does_on_mail_of_known_topics() {
    for n in [2_000, 10_000, 50_000] {
        measure(n);
    }
}
