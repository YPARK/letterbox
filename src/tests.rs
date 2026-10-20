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
    account.password_command = Some("echo no such entry >&2; exit 1".to_string());
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
    assert!(m.text.contains("Note G"));
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

/// Where a test keeps a password: in no keychain, but a command printing it.
fn keep_in_test(_: Keeper, _: &str, _: &str, password: &str) -> anyhow::Result<Option<String>> {
    Ok(Some(format!("echo {password}")))
}

fn empty_app(config: PathBuf) -> App {
    let mut app = App::new(Vec::new(), config);
    app.keep = keep_in_test;
    app.keepers = || vec![Keeper::Keychain, Keeper::Gpg];
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
    assert!(
        file.contains("password_command = \"echo secret\""),
        "{file}"
    );
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
    assert_eq!(removed.host, "imap.b.c");
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
    click_at(&mut app, app::Target::Field(6));
    click_at(&mut app, app::Target::Field(6));
    assert_eq!(
        app.form.as_ref().unwrap().keeper,
        1,
        "a second click picks gpg"
    );
    click_at(&mut app, app::Target::Field(5));
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

// ---- gpg ----

#[cfg(unix)]
#[test]
fn a_password_kept_with_gpg_is_encrypted_and_found_again() {
    if std::process::Command::new("gpg")
        .arg("--version")
        .output()
        .is_err()
    {
        eprintln!("no gpg here: not tried");
        return;
    }
    let home = tempfile::Builder::new()
        .prefix("lbg")
        .tempdir_in("/tmp")
        .unwrap();
    let data = tempfile::tempdir().unwrap();
    // SAFETY: tests that read these are this one alone.
    unsafe {
        std::env::set_var("GNUPGHOME", home.path());
        std::env::set_var("XDG_DATA_HOME", data.path());
    }
    let made = std::process::Command::new("gpg")
        .args(["--batch", "--passphrase", "", "--quick-generate-key"])
        .args([
            "letterbox test <test@example.com>",
            "default",
            "default",
            "never",
        ])
        .output()
        .unwrap();
    assert!(
        made.status.success(),
        "{}",
        String::from_utf8_lossy(&made.stderr)
    );
    let command =
        crate::secret::store(Keeper::Gpg, "imap.example.com", "me@example.com", "p'w \"d")
            .unwrap()
            .expect("a command to decrypt it");
    let file = data
        .path()
        .join("letterbox/passwords/me@example.com@imap.example.com.gpg");
    let bytes = std::fs::read(&file).unwrap();
    assert!(
        !String::from_utf8_lossy(&bytes).contains("p'w"),
        "encrypted"
    );
    let mut account = crate::setup::account("W", "me@example.com", "imap.example.com", 993, "");
    account.password_command = Some(command.clone());
    assert_eq!(account.password().unwrap(), "p'w \"d");
    assert!(crate::secret::asks(&command));
    crate::secret::forget("imap.example.com", "me@example.com", Some(&command));
    assert!(!file.exists());
    let _ = std::process::Command::new("gpgconf")
        .args(["--kill", "gpg-agent"])
        .status();
}
