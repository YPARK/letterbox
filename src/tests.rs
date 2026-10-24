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
    flags: Vec<&'static str>,
    raw: Vec<u8>,
}

struct Mailbox {
    name: String,
    attributes: &'static str,
    letters: Vec<Letter>,
}

type Boxes = Arc<Mutex<Vec<Mailbox>>>;

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

fn boxes() -> Boxes {
    let inbox = vec![
        Letter {
            uid: 11,
            flags: vec!["\\Seen"],
            raw: raw(
                "Rosalind Franklin <rf@kings.example>",
                "Photo 51",
                "Content-Type: text/plain; charset=utf-8\r\n",
                "The B form, at last.\r\n",
            ),
        },
        Letter {
            uid: 12,
            flags: vec!["\\Seen", "\\Flagged"],
            raw: raw(
                "Alan Turing <alan@bletchley.example>",
                "=?utf-8?q?Computable_numbers_=E2=80=94_the_paper?=",
                "Content-Type: multipart/mixed; boundary=\"m1\"\r\n",
                "--m1\r\nContent-Type: text/plain; charset=utf-8\r\n\r\n\
                 > what you asked\r\nThe paper is attached.\r\n\r\n--m1\r\n\
                 Content-Type: text/plain; name=\"states.txt\"\r\n\
                 Content-Disposition: attachment; filename=\"../states.txt\"\r\n\r\n\
                 b None P0 c\r\n--m1--\r\n",
            ),
        },
        Letter {
            uid: 14,
            flags: vec![],
            raw: raw(
                "=?utf-8?q?Emmy_N=C3=B6ther?= <noether@goettingen.example>",
                "Symmetry\x1b[2J",
                "Content-Type: text/plain; charset=utf-8\r\n",
                "Every symmetry has its conserved quantity.\x1b]0;owned\x07\r\n",
            ),
        },
    ];
    Arc::new(Mutex::new(vec![
        Mailbox {
            name: "INBOX".to_string(),
            attributes: "\\HasNoChildren",
            letters: inbox,
        },
        Mailbox {
            name: "Projects".to_string(),
            attributes: "\\HasChildren",
            letters: Vec::new(),
        },
        Mailbox {
            name: "Projects/Grant".to_string(),
            attributes: "\\HasNoChildren",
            letters: Vec::new(),
        },
        Mailbox {
            name: "Deleted Items".to_string(),
            attributes: "\\HasNoChildren",
            letters: Vec::new(),
        },
        Mailbox {
            name: "Sent Items".to_string(),
            attributes: "\\HasNoChildren \\Sent",
            letters: Vec::new(),
        },
        Mailbox {
            name: "Entw&APw-rfe".to_string(),
            attributes: "\\HasNoChildren",
            letters: Vec::new(),
        },
        Mailbox {
            name: "Drafts".to_string(),
            attributes: "\\HasNoChildren \\Drafts",
            letters: Vec::new(),
        },
    ]))
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
    boxes
        .iter()
        .flat_map(|b| b.letters.iter().map(|l| l.uid))
        .max()
        .unwrap_or(0)
        + 1
}

/// The byte count of an IMAP literal, as `{310}` at the end of a command.
fn literal_size(args: &str) -> usize {
    args.rsplit_once('{')
        .and_then(|(_, n)| n.trim_end_matches('}').parse().ok())
        .unwrap_or(0)
}

/// The numbers in an IMAP set, as `1:3,7`.
fn set(s: &str) -> Vec<u32> {
    s.split(',')
        .flat_map(|part| match part.split_once(':') {
            Some((a, b)) => (a.parse().unwrap_or(0)..=b.parse().unwrap_or(0)).collect(),
            None => part.parse().into_iter().collect::<Vec<u32>>(),
        })
        .collect()
}

fn fetched(seq: usize, l: &Letter, items: &str) -> Vec<u8> {
    let mut out = format!("* {seq} FETCH (UID {}", l.uid).into_bytes();
    if items.contains("BODY.PEEK[]") {
        out.extend(format!(" BODY[] {{{}}}\r\n", l.raw.len()).bytes());
        out.extend(&l.raw);
    } else {
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
    }
    out.extend(b")\r\n");
    out
}

fn serve(tcp: TcpStream, shared: Boxes) {
    let mut out = tcp.try_clone().unwrap();
    let mut lines = BufReader::new(tcp);
    let mut selected: Option<usize> = None;
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
            "SELECT" | "EXAMINE" => {
                let name = &words(args)[0];
                match boxes.iter().position(|b| b.name == *name) {
                    Some(i) => {
                        selected = Some(i);
                        reply.extend(
                            format!(
                                "* FLAGS (\\Seen \\Flagged)\r\n* {} EXISTS\r\n* 0 RECENT\r\n\
                                 * OK [UIDVALIDITY 7] ok\r\n",
                                boxes[i].letters.len()
                            )
                            .bytes(),
                        );
                        done = format!("{tag} OK [READ-WRITE] selected\r\n");
                    }
                    None => done = format!("{tag} NO no such folder\r\n"),
                }
            }
            "STATUS" => {
                let name = &words(args)[0];
                let b = boxes.iter().find(|b| b.name == *name).unwrap();
                let unseen = b
                    .letters
                    .iter()
                    .filter(|l| !l.flags.contains(&"\\Seen"))
                    .count();
                reply.extend(
                    format!(
                        "* STATUS \"{}\" (MESSAGES {} UNSEEN {unseen})\r\n",
                        b.name,
                        b.letters.len()
                    )
                    .bytes(),
                );
            }
            "FETCH" => {
                let (seqs, items) = args.split_once(' ').unwrap();
                let b = &boxes[selected.unwrap()];
                for seq in set(seqs) {
                    if let Some(l) = b.letters.get(seq as usize - 1) {
                        reply.extend(fetched(seq as usize, l, items));
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
                        let uids = set(uids);
                        for (i, l) in b.letters.iter().enumerate() {
                            if uids.contains(&l.uid) {
                                reply.extend(fetched(i + 1, l, items));
                            }
                        }
                    }
                    "STORE" => {
                        let b = &mut boxes[selected.unwrap()];
                        let w = words(args);
                        let (uid, how, flag) = (w[0].parse::<u32>().unwrap(), &w[1], &w[2]);
                        let flag: &'static str = if flag.contains("Seen") {
                            "\\Seen"
                        } else if flag.contains("Flagged") {
                            "\\Flagged"
                        } else if flag.contains("Deleted") {
                            "\\Deleted"
                        } else if flag.contains("Answered") {
                            "\\Answered"
                        } else {
                            "\\Seen"
                        };
                        if let Some(l) = b.letters.iter_mut().find(|l| l.uid == uid) {
                            l.flags.retain(|f| *f != flag);
                            if how.starts_with('+') {
                                l.flags.push(flag);
                            }
                        }
                    }
                    "SEARCH" => {
                        let b = &boxes[selected.unwrap()];
                        let w = words(args);
                        let all = w.iter().any(|x| x.eq_ignore_ascii_case("ALL"))
                            && !w.iter().any(|x| {
                                x.eq_ignore_ascii_case("TEXT") || x.eq_ignore_ascii_case("HEADER")
                            });
                        let wanted = w.last().unwrap().to_lowercase();
                        let uids: Vec<String> = b
                            .letters
                            .iter()
                            .filter(|l| {
                                all || String::from_utf8_lossy(&l.raw)
                                    .to_lowercase()
                                    .contains(&wanted)
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
                            boxes[at].letters.push(Letter {
                                uid: new_uid,
                                flags: letter.flags,
                                raw: letter.raw,
                            });
                            done = format!("{tag} OK [COPYUID 7 {uid} {new_uid}] moved\r\n");
                        }
                    }
                    "EXPUNGE" => {
                        let b = &mut boxes[selected.unwrap()];
                        let uids = set(words(args).first().map(String::as_str).unwrap_or(""));
                        let mut seqs = Vec::new();
                        for (i, letter) in b.letters.iter().enumerate() {
                            if uids.contains(&letter.uid) && letter.flags.contains(&"\\Deleted") {
                                seqs.push(i + 1);
                            }
                        }
                        b.letters
                            .retain(|l| !(uids.contains(&l.uid) && l.flags.contains(&"\\Deleted")));
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
                    boxes.push(Mailbox {
                        name,
                        attributes: "\\HasNoChildren",
                        letters: Vec::new(),
                    });
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
                let flags: Vec<&'static str> = if args.contains("\\Seen") {
                    vec!["\\Seen"]
                } else {
                    Vec::new()
                };
                match boxes.iter_mut().find(|b| b.name == name) {
                    Some(b) => b.letters.push(Letter {
                        uid: new_uid,
                        flags,
                        raw: body,
                    }),
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
                reply.extend(b"* CAPABILITY IMAP4rev1 MOVE UIDPLUS\r\n");
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
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let port = listener.local_addr().unwrap().port();
    let boxes = boxes();
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

fn unseen(app: &App) -> Option<u32> {
    app.folder().and_then(|f| f.unseen)
}

#[test]
fn the_inbox_is_listed_first() {
    let app = demo();
    assert_eq!(app.folder_name(), "Inbox");
    assert_eq!(app.shown.len(), 6);
    assert_eq!(app.shown[0].from, "Ada Lovelace");
    assert_eq!(unseen(&app), Some(2));
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
    assert_eq!(unseen(&app), Some(1));
    press(&mut app, &["u"]);
    assert!(!app.shown[0].flags.seen);
    assert_eq!(unseen(&app), Some(2));
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
    assert_eq!(app.message().unwrap().subject, "Re: the compiler");
    press(&mut app, &["<esc>", "/", "photo"]);
    assert_eq!(app.shown.len(), 1);
    press(&mut app, &["<esc>"]);
    assert_eq!(app.shown.len(), 6);
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
    press(&mut app, &["G", "k", "k", "<enter>"]);
    assert_eq!(
        app.message().unwrap().subject,
        "Computable numbers: the paper, and the figures"
    );
    press(&mut app, &["a", "a"]);
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
    press(&mut app, &["f"]);
    let files = app.dialog.files().expect("listed");
    assert_eq!(files.items.len(), 2, "two attachments, no links");
    press(&mut app, &["j", "s"]);
    assert!(dir.path().join("computable-numbers (3).pdf").is_file());

    // ⏎ hands a PDF to ohp, and another file to the system; the mouse too.
    press(&mut app, &["<enter>"]);
    let Some(app::Launch::Ohp(pdf)) = app.launch.take() else {
        panic!("not for ohp: {:?}", app.launch);
    };
    assert!(std::fs::read(&pdf).unwrap().starts_with(b"%PDF"));
    assert!(pdf.starts_with(cache.path()));
    click_at(&mut app, app::Target::Item(0));
    let Some(app::Launch::Open(text)) = app.launch.take() else {
        panic!("not for the system: {:?}", app.launch);
    };
    assert_eq!(text.file_name().unwrap(), "3-states.txt");
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
    press(&mut app, &["A", "me@ubc.ca", "<c-o>"]);
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
    let mut work = crate::setup::account("Work", "me@ubc.ca", "imap.ubc.ca", 993, "me@ubc.ca");
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
    assert_eq!(guess("x@ubc.ca"), ("imap.ubc.ca".to_string(), None));
    assert_eq!(guess("nothing").0, "");
    assert_eq!(crate::config::guess_smtp("x@gmail.com"), "smtp.gmail.com");
    assert_eq!(
        crate::config::guess_smtp("x@outlook.com"),
        "smtp.office365.com"
    );
    assert_eq!(crate::config::guess_smtp("x@ubc.ca"), "smtp.ubc.ca");
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
    assert_eq!(page("x@ubc.ca", "imap.gmail.com"), page("x@gmail.com", ""));
    assert_eq!(
        page("x@outlook.com", ""),
        None,
        "it signs in in the browser"
    );
    assert_eq!(page("x@ubc.ca", "imap.ubc.ca"), None);
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
        app_password("imap.ubc.ca", "abcd efgh ijkl mnop"),
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
/// pointed at in every test: an address at `ubc.example` is its own; a code
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
        let ours = target.contains("ubc.example");
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
    press(&mut app, &["A", "me@ubc.example", "<tab>"]);
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
    assert_eq!(app.status, "added ubc.example");
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
        "me@ubc.example",
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
        "me@ubc.example",
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
    let mut a = crate::setup::account("O", "me@ubc.example", "outlook.example", 993, "me");
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
        (Some("INBOX"), Some(6))
    );

    let listed = call(&mut server, "list_messages", json!({ "limit": 2 }));
    let listed = &listed["structuredContent"];
    assert_eq!(listed["shown"], 2);
    assert_eq!(listed["more"], true);
    assert_eq!(listed["messages"][0]["from"], "Ada Lovelace");
    let uid = listed["messages"][0]["uid"].as_u64().unwrap();

    let found = call(&mut server, "search_messages", json!({ "query": "photo" }));
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
    press(&mut app, &["<c-g>"]);
    assert!(app.compose.is_none());
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
    let mut work = Demo::new();
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
            ("Home".to_string(), Box::new(Demo::new()) as Box<dyn Store>),
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
    assert_eq!(app.unseen_of(Role::Inbox), Some(2 + 3));
    list_row(&mut app, app::Row::All(Role::Inbox));
    assert_eq!(app.shown.len(), 13, "{}", app.status);
    assert_eq!(app.shown[0].subject, "Newest of all");
    assert_eq!(app.shown[0].account, 1);
    assert_eq!(app.shown.iter().filter(|m| m.account == 0).count(), 6);
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
    assert_eq!(unseen(&app, 1), Some(2), "Work's newest is read");
    assert_eq!(unseen(&app, 0), Some(2), "Home's are not");
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
    assert_eq!(app.shown.len(), 12);
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
    // r is not reply: it says what is.
    press(&mut app, &["r"]);
    assert!(app.compose.is_none());
    assert!(app.status.contains("C-r"), "{}", app.status);
    press(&mut app, &["<c-n>"]);
    assert!(app.status.contains("new mail"), "{}", app.status);
    press(&mut app, &["<c-r>"]);
    let text = app.compose.as_ref().unwrap().buffer.text();
    assert!(text.contains("Subject: Re: "), "{text}");
    press(&mut app, &["<c-g>", "e"]);
    assert!(app.compose.is_some(), "e replies to all");
    press(&mut app, &["<c-g>", "<c-s-r>"]);
    assert!(app.compose.is_some(), "so does C-S-r");
    press(&mut app, &["<c-g>", "<c-f>"]);
    let text = app.compose.as_ref().unwrap().buffer.text();
    assert!(text.contains("Subject: Fwd: "), "{text}");
    press(&mut app, &["<c-g>"]);
    let help = {
        press(&mut app, &["?"]);
        app::screen(&mut app, 100, 60)
    };
    for key in ["C-r", "C-S-r", "C-f", "C-n", "R"] {
        assert!(help.contains(key), "{key} in {help}");
    }
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

    // C-g lets the mark go before it leaves the message.
    let mut buffer = Buffer::from_text("x");
    assert_eq!(edit_keys(&mut buffer, &["<c- >", "<c-g>"]), Done::Stay);
    assert_eq!(buffer.region(), None);
    assert_eq!(edit_keys(&mut buffer, &["<c-g>"]), Done::Cancel);
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
    // Shift's letter, as terminals say it or not.
    let ctrl_shift = KeyModifiers::CONTROL | KeyModifiers::SHIFT;
    for key in [
        KeyEvent::new(KeyCode::Char('r'), ctrl_shift),
        KeyEvent::new(KeyCode::Char('R'), ctrl_shift),
        KeyEvent::new(KeyCode::Char('R'), KeyModifiers::CONTROL),
    ] {
        assert_eq!(
            crate::keymap::act(key),
            crate::keymap::act(key_named("e").unwrap()),
        );
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
    press(&mut app, &["<enter>", "f"]);
    let _ = app::screen(&mut app, 120, 40);
    for _ in 0..3 {
        app.mouse(mouse(MouseEventKind::ScrollDown, 1, 1));
    }
    assert_eq!(app.dialog.files().expect("listed").at, 1, "the last of two");
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
fn r_says_the_keys_the_table_gives_reply() {
    let mut app = demo();
    press(&mut app, &["r"]);
    for name in crate::keymap::keys_for(crate::keymap::Act::ReplyAll) {
        assert!(app.status.contains(name), "{name} in {}", app.status);
    }
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
