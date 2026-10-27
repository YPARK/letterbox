//! An IMAP server, just enough of one, on this machine: what the tests of
//! IMAP and the map's sync are served by.

use super::*;

// ---- an IMAP server, just enough of one, on this machine ----

#[derive(Clone)]
pub(super) struct Letter {
    pub(super) uid: u32,
    pub(super) flags: Vec<String>,
    pub(super) raw: Vec<u8>,
    /// When it last changed, as CONDSTORE counts.
    pub(super) modseq: u64,
    /// Gmail's labels on it, and its ID there.
    pub(super) labels: Vec<String>,
    pub(super) msgid: u64,
}

impl Letter {
    pub(super) fn new(uid: u32, flags: &[&str], raw: Vec<u8>) -> Letter {
        Letter {
            uid,
            flags: flags.iter().map(|f| f.to_string()).collect(),
            raw,
            modseq: 1,
            labels: Vec::new(),
            msgid: 0,
        }
    }

    pub(super) fn has(&self, flag: &str) -> bool {
        self.flags.iter().any(|f| f.eq_ignore_ascii_case(flag))
    }
}

pub(super) struct Mailbox {
    pub(super) name: String,
    pub(super) attributes: &'static str,
    pub(super) letters: Vec<Letter>,
    pub(super) validity: u32,
    /// The UID the next message will have.
    pub(super) next: u32,
}

impl Mailbox {
    pub(super) fn new(name: &str, attributes: &'static str, letters: Vec<Letter>) -> Mailbox {
        let next = letters.iter().map(|l| l.uid + 1).max().unwrap_or(1);
        Mailbox {
            name: name.to_string(),
            attributes,
            letters,
            validity: 7,
            next,
        }
    }

    pub(super) fn push(&mut self, letter: Letter) {
        self.next = self.next.max(letter.uid + 1);
        self.letters.push(letter);
    }
}

/// How the fake server keeps labels, as servers do.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(super) enum Mode {
    /// In folders alone, as Microsoft's: no keywords, no CONDSTORE.
    Folders,
    /// In keywords of any name, with CONDSTORE, as Dovecot's and Fastmail's.
    Keywords,
    /// As Gmail's labels, with its IDs, and CONDSTORE.
    Gmail,
}

/// The server's mail, how it keeps labels, its mod-sequence, and the
/// commands it was sent.
pub(super) struct Fake {
    pub(super) boxes: Vec<Mailbox>,
    pub(super) mode: Mode,
    pub(super) modseq: u64,
    pub(super) said: Vec<String>,
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
    pub(super) fn tick(&mut self) -> u64 {
        self.modseq += 1;
        self.modseq
    }
}

pub(super) type Boxes = Arc<Mutex<Fake>>;

pub(super) fn raw(from: &str, subject: &str, extra: &str, body: &str) -> Vec<u8> {
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

pub(super) fn fake(boxes: Vec<Mailbox>, mode: Mode) -> Boxes {
    Arc::new(Mutex::new(Fake {
        boxes,
        mode,
        modseq: 1,
        said: Vec::new(),
    }))
}

pub(super) fn boxes() -> Boxes {
    boxes_in(Mode::Folders)
}

pub(super) fn boxes_in(mode: Mode) -> Boxes {
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
pub(super) fn gmail_boxes() -> Boxes {
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
pub(super) fn words(s: &str) -> Vec<String> {
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

pub(super) fn next_uid(boxes: &[Mailbox]) -> u32 {
    boxes.iter().map(|b| b.next).max().unwrap_or(1)
}

/// The byte count of an IMAP literal, as `{310}` at the end of a command.
pub(super) fn literal_size(args: &str) -> usize {
    args.rsplit_once('{')
        .and_then(|(_, n)| n.trim_end_matches('}').parse().ok())
        .unwrap_or(0)
}

/// The ranges of an IMAP set, as `1:3,7` or `5:*`.
pub(super) fn set(s: &str) -> Vec<(u32, u32)> {
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

pub(super) fn within(ranges: &[(u32, u32)], n: u32) -> bool {
    ranges.iter().any(|&(a, b)| a <= n && n <= b)
}

pub(super) fn fetched(seq: usize, l: &Letter, items: &str, changed: bool) -> Vec<u8> {
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
pub(super) fn listed(s: &str) -> Vec<String> {
    words(s.trim().trim_start_matches('(').trim_end_matches(')'))
}

pub(super) fn serve(tcp: TcpStream, shared: Boxes) {
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
pub(super) fn server() -> (Account, Boxes) {
    serve_boxes(boxes())
}

/// A server on this machine with the mail `boxes` holds, and an account
/// on it.
pub(super) fn serve_boxes(boxes: Boxes) -> (Account, Boxes) {
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
