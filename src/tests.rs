use crate::app::{self, App, Body, Focus};
use crate::config::{Account, Security};
use crate::demo::Demo;
use crate::imap::Imap;
use crate::mail::{self, Folder, Role};
use crate::secret::Keeper;
use crate::store::{Flag, Store};
use crate::ui;
use std::io::{BufRead, BufReader, Write};
use std::net::{TcpListener, TcpStream};
use std::path::PathBuf;
use std::sync::{Arc, Mutex};
use std::time::Duration;

// ---- an IMAP server, just enough of one, on this machine ----

struct Letter {
    uid: u32,
    flags: Vec<&'static str>,
    raw: Vec<u8>,
}

struct Mailbox {
    name: &'static str,
    attributes: &'static str,
    letters: Vec<Letter>,
}

type Boxes = Arc<Mutex<Vec<Mailbox>>>;

fn raw(from: &str, subject: &str, extra: &str, body: &str) -> Vec<u8> {
    format!(
        "From: {from}\r\nTo: me@example.com\r\nSubject: {subject}\r\n\
         Date: Tue, 20 Oct 2026 10:00:00 +0000\r\nMIME-Version: 1.0\r\n{extra}\r\n{body}"
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
            name: "INBOX",
            attributes: "\\HasNoChildren",
            letters: inbox,
        },
        Mailbox {
            name: "Projects",
            attributes: "\\HasChildren",
            letters: Vec::new(),
        },
        Mailbox {
            name: "Projects/Grant",
            attributes: "\\HasNoChildren",
            letters: Vec::new(),
        },
        Mailbox {
            name: "Deleted Items",
            attributes: "\\HasNoChildren",
            letters: Vec::new(),
        },
        Mailbox {
            name: "Sent Items",
            attributes: "\\HasNoChildren \\Sent",
            letters: Vec::new(),
        },
        Mailbox {
            name: "Entw&APw-rfe",
            attributes: "\\HasNoChildren",
            letters: Vec::new(),
        },
        Mailbox {
            name: "Drafts",
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

fn serve(tcp: TcpStream, boxes: Boxes) {
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
        let mut boxes = boxes.lock().unwrap();
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
                match boxes.iter().position(|b| b.name == name) {
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
                let b = boxes.iter().find(|b| b.name == name).unwrap();
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
                let b = &mut boxes[selected.unwrap()];
                match sub.to_uppercase().as_str() {
                    "FETCH" => {
                        let (uids, items) = args.split_once(' ').unwrap();
                        let uids = set(uids);
                        for (i, l) in b.letters.iter().enumerate() {
                            if uids.contains(&l.uid) {
                                reply.extend(fetched(i + 1, l, items));
                            }
                        }
                    }
                    "STORE" => {
                        let w = words(args);
                        let (uid, how, flag) = (w[0].parse::<u32>().unwrap(), &w[1], &w[2]);
                        let flag: &'static str = if flag.contains("Seen") {
                            "\\Seen"
                        } else {
                            "\\Flagged"
                        };
                        let l = b.letters.iter_mut().find(|l| l.uid == uid).unwrap();
                        l.flags.retain(|f| *f != flag);
                        if how.starts_with('+') {
                            l.flags.push(flag);
                        }
                    }
                    "SEARCH" => {
                        let w = words(args);
                        let wanted = w.last().unwrap().to_lowercase();
                        let uids: Vec<String> = b
                            .letters
                            .iter()
                            .filter(|l| {
                                String::from_utf8_lossy(&l.raw)
                                    .to_lowercase()
                                    .contains(&wanted)
                            })
                            .map(|l| l.uid.to_string())
                            .collect();
                        reply.extend(format!("* SEARCH {}\r\n", uids.join(" ")).bytes());
                    }
                    _ => done = format!("{tag} BAD unknown\r\n"),
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
            "NOOP" | "CAPABILITY" => {}
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
    app.settle(Duration::from_secs(5));
    app
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
    let form = app.form.as_ref().unwrap();
    assert_eq!(form.fields[1], "example.com");
    assert_eq!(form.fields[2], "imap.example.com");
    assert_eq!(form.fields[4], "me@example.com");
    press(&mut app, &["<esc>"]);
    assert!(app.form.is_none());

    fill_form(&mut app, account.port.unwrap(), "secret");
    assert!(
        app.form.is_none(),
        "{:?}",
        app.form.as_ref().and_then(|f| f.note.clone())
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
    let form = app.form.as_ref().expect("the form stays open");
    assert!(form.failed);
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

#[test]
fn the_demos_account_is_not_changed() {
    let mut app = demo();
    app.fix_accounts();
    press(&mut app, &["A"]);
    assert!(app.form.is_none());
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

use ratatui::crossterm::event::{KeyCode, KeyModifiers, MouseButton, MouseEvent, MouseEventKind};

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
    assert_eq!(app.form.as_ref().unwrap().at, 2);
    press(&mut app, &["<c-u>", "127.0.0.1", "<tab>", "<c-u>"]);
    press(&mut app, &[&account.port.unwrap().to_string()]);
    click_at(&mut app, app::Target::Field(4));
    press(&mut app, &["<c-u>", "me"]);
    click_at(&mut app, app::Target::Field(7));
    click_at(&mut app, app::Target::Field(7));
    assert_eq!(
        app.form.as_ref().unwrap().keeper,
        1,
        "a second click picks the vault"
    );
    click_at(&mut app, app::Target::Field(7));
    assert_eq!(
        app.form.as_ref().unwrap().keeper,
        0,
        "and a third the keyring again"
    );
    click_at(&mut app, app::Target::Field(6));
    press(&mut app, &["secret"]);
    click_at(&mut app, app::Target::Button(app::Button::Add));
    assert!(
        app.form.is_none(),
        "{:?}",
        app.form.as_ref().and_then(|f| f.note.clone())
    );
    assert_eq!(crate::config::load(&path).unwrap().len(), 1);

    press(&mut app, &["h", "D"]);
    click_at(&mut app, app::Target::Button(app::Button::Keep));
    assert!(app.removing.is_none());
    assert_eq!(crate::config::load(&path).unwrap().len(), 1);
    press(&mut app, &["D"]);
    click_at(&mut app, app::Target::Button(app::Button::Remove));
    assert!(crate::config::load(&path).unwrap().is_empty());
}

#[test]
fn the_keys_at_the_foot_are_clicked_and_the_help_scrolls() {
    let mut app = demo();
    click_at(&mut app, app::Target::Key(KeyCode::Char('?')));
    assert!(app.help);
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
    assert_eq!(app.help_scroll, 3);
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
    assert!(!app.help);
    click_at(&mut app, app::Target::Key(KeyCode::Enter));
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
    let form = app.form.as_ref().unwrap();
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
    let form = app.form.as_ref().expect("still open");
    assert_eq!(form.note.as_deref(), Some("the two passphrases differ"));
    assert!(!vault_at.exists());
    press(&mut app, &["<c-u>", "correct horse", "<enter>"]);
    assert!(
        app.form.is_none(),
        "{:?}",
        app.form.as_ref().and_then(|f| f.note.clone())
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
    let form = app.form.as_ref().unwrap();
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
        app.form.is_none(),
        "{:?}",
        app.form.as_ref().and_then(|f| f.note.clone())
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
