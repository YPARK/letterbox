//! What the map reads of a server, mail read, and the app on the demo's
//! mail.

use super::*;

// ---- what the map reads of a server ----

/// A message in `folder` of the fake server, with `flags`, from a list.
pub(super) fn add_listed(boxes: &Boxes, folder: &str, flags: &[&str], subject: &str) -> u32 {
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
    let (flags, keywords) =
        crate::imap::flags_named(["\\Seen", "Grant", "$Forwarded", "NonJunk", "NotJunk"]);
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

pub(super) fn demo() -> App {
    let mut app = App::new(
        vec![("Demo".to_string(), Box::new(Demo::new()) as Box<dyn Store>)],
        PathBuf::from("/nowhere/accounts.toml"),
    );
    // A ledger of this test's own, not the one letterbox keeps for itself.
    app.ledger = scratch_ledger();
    app.settle(Duration::from_secs(5));
    app
}

pub(super) fn scratch_ledger() -> crate::trash::Ledger {
    static N: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
    let n = N.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
    let path =
        std::env::temp_dir().join(format!("letterbox-trash-{}-{n}.toml", std::process::id()));
    let _ = std::fs::remove_file(&path);
    crate::trash::Ledger::open(path)
}

pub(super) fn press(app: &mut App, keys: &[&str]) {
    for key in keys.iter().flat_map(|k| app::keys(k)) {
        let _ = app::screen(app, 140, 40);
        app.key(key);
        app.settle(Duration::from_secs(5));
    }
}

/// The demo's Inbox: the six letters, and the lab's mail still to deal with;
/// and how many of them are unread.
pub(super) const INBOX: usize = 33;
/// How many the demo has in Archive: what is read and old, and a Scholar
/// alert.
pub(super) const ARCHIVE: usize = 21;
pub(super) const UNREAD: u32 = 18;

pub(super) fn unseen(app: &App) -> Option<u32> {
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
    // All of them, twice, from what o opens: S, into the folder picked,
    // begun on the downloads folder.
    press(&mut app, &["o", "S", "<enter>", "o", "S", "<enter>"]);
    assert!(dir.path().join("states.txt").is_file());
    assert!(
        dir.path().join("states (2).txt").is_file(),
        "{}",
        app.status
    );
    assert!(dir.path().join("computable-numbers (2).pdf").is_file());

    // One of them, from the list of its files and links.
    let cache = tempfile::tempdir().unwrap();
    let _held = CACHE_AT.lock().unwrap_or_else(|e| e.into_inner());
    unsafe { std::env::set_var("XDG_CACHE_HOME", cache.path()) };
    press(&mut app, &["o"]);
    let files = app.dialog.files().expect("listed");
    assert_eq!(
        files.items.len(),
        4,
        "the message, in the browser and in ohp, two attachments, no links"
    );
    assert_eq!(files.at, 2, "begun on the first file");
    press(&mut app, &["j", "s", "<enter>", "<enter>"]);
    assert!(dir.path().join("computable-numbers (3).pdf").is_file());
    assert!(app.dialog.files().is_some(), "back to its files");

    // ⏎ hands a PDF to ohp, and another file to the system; the mouse too.
    press(&mut app, &["<enter>"]);
    let Some(app::Launch::Ohp(pdf)) = app.launch.take() else {
        panic!("not for ohp: {:?}", app.launch);
    };
    assert!(std::fs::read(&pdf).unwrap().starts_with(b"%PDF"));
    assert!(pdf.starts_with(cache.path()));
    click_at(&mut app, app::Target::Item(2));
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
fn the_message_read_is_presented_by_ohp_with_the_images_it_carries() {
    let cache = tempfile::tempdir().unwrap();
    let _held = CACHE_AT.lock().unwrap_or_else(|e| e.into_inner());
    unsafe { std::env::set_var("XDG_CACHE_HOME", cache.path()) };
    let mut app = demo();
    // Nothing read: nothing to present.
    press(&mut app, &["O"]);
    assert!(app.launch.is_none());
    press(&mut app, &["<enter>", "O"]);
    let Some(app::Launch::Ohp(page)) = app.launch.take() else {
        panic!("not for ohp: {:?}", app.launch);
    };
    assert!(page.starts_with(cache.path()));
    assert!(app::ohp_reads(&page.to_string_lossy()));
    let html = std::fs::read_to_string(&page).unwrap();
    assert!(html.contains("script-src 'none'"), "{html}");
    // From the list of its files too.
    press(&mut app, &["o"]);
    let at = app
        .dialog
        .files()
        .unwrap()
        .items
        .iter()
        .position(|i| *i == app::Item::Present)
        .unwrap();
    click_at(&mut app, app::Target::Item(at));
    assert!(matches!(app.launch.take(), Some(app::Launch::Ohp(_))));
    unsafe { std::env::remove_var("XDG_CACHE_HOME") };
    // An image the HTML shows by cid: is found by its Content-ID.
    let raw = b"From: a@b.c\r\nSubject: logo\r\nMIME-Version: 1.0\r\n\
Content-Type: multipart/related; boundary=\"x\"\r\n\r\n\
--x\r\nContent-Type: text/html\r\n\r\n<img src=\"cid:logo@b.c\">\r\n\
--x\r\nContent-Type: image/png\r\nContent-ID: <logo@b.c>\r\n\
Content-Transfer-Encoding: base64\r\n\r\niVBORw0KGgo=\r\n--x--\r\n";
    let images = crate::mail::inline_images(raw);
    assert_eq!(images.len(), 1, "{images:?}");
    assert_eq!(images[0].0, "logo@b.c");
    assert_eq!(images[0].1, "image/png");
    assert!(images[0].2.starts_with(b"\x89PNG"));
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

#[test]
fn an_attachment_is_saved_into_the_folder_picked_and_the_next_begins_there() {
    let dir = tempfile::tempdir().unwrap();
    std::fs::create_dir(dir.path().join("papers")).unwrap();
    std::fs::create_dir(dir.path().join("photos")).unwrap();
    std::fs::write(dir.path().join("notes.txt"), b"x").unwrap();
    let mut app = demo();
    app.save_dir = Some(dir.path().to_path_buf());
    let turing = app
        .shown
        .iter()
        .position(|m| m.from == "Alan Turing")
        .unwrap();
    app.list.select(Some(turing));
    press(&mut app, &["<enter>", "o", "j", "s"]);
    // Its folders alone, to save in, begun on saving here.
    let Some(p) = app.dialog.saving() else {
        panic!("no folder to pick");
    };
    assert_eq!(p.dir, dir.path());
    let names: Vec<&str> = p.shown().iter().map(|e| e.name()).collect();
    assert_eq!(names, ["..", ".", "papers", "photos"]);
    assert_eq!(p.current(), Some(&crate::pick::Entry::Here));
    let screen = app::screen(&mut app, 120, 40);
    assert!(
        screen.contains("Save to") && screen.contains("save here"),
        "{screen}"
    );

    // Esc, and back to its files, nothing saved.
    press(&mut app, &["<esc>"]);
    assert!(app.dialog.files().is_some());
    assert!(!dir.path().join("computable-numbers.pdf").exists());

    // Typing narrows to a folder, ⏎ goes in, and ⏎ again saves there.
    press(&mut app, &["s", "pap", "<enter>", "<enter>", "<enter>"]);
    let saved = dir.path().join("papers").join("computable-numbers.pdf");
    assert!(saved.is_file(), "{:?}", app.dialog.files().map(|f| &f.note));
    assert_eq!(
        app.save_dir.as_deref(),
        Some(dir.path().join("papers").as_path())
    );

    // The next begins where the last was saved; S saves all there.
    press(&mut app, &["S"]);
    assert_eq!(
        app.dialog.saving().map(|p| p.dir.clone()),
        Some(dir.path().join("papers"))
    );
    press(&mut app, &["<enter>"]);
    assert!(dir.path().join("papers").join("states.txt").is_file());
    assert!(!app.dialog.is_open(), "{}", app.status);

    // Kept for next time, as how the mail is looked at is.
    let look = dir.path().join("look");
    app.look_file = Some(look.clone());
    app.remember_look();
    let mut again = demo();
    again.look_file = Some(look);
    again.recall_look();
    assert_eq!(again.save_dir, Some(dir.path().join("papers")));
}

#[test]
fn a_path_typed_to_save_in_is_saved_in_and_a_name_new_there_made() {
    use crate::pick::{Entry, Picked, Picker};
    let key = |p: &mut Picker, s: &str| {
        let mut last = Picked::Stay;
        for k in app::keys(s) {
            last = p.key(k);
        }
        last
    };
    let dir = tempfile::tempdir().unwrap();
    std::fs::create_dir_all(dir.path().join("docs/archive")).unwrap();
    let docs = dir.path().join("docs");

    // A path typed to its folder's `/` stays on saving there: a path is
    // typed from `/`, as no Windows one begins.
    if cfg!(unix) {
        let mut p = Picker::folder(dir.path().to_path_buf());
        key(&mut p, &format!("{}/", docs.display()));
        assert_eq!(p.dir, docs);
        assert_eq!(p.current(), Some(&Entry::Here));
        assert_eq!(key(&mut p, "<enter>"), Picked::Folder(docs.clone()));
    }

    // A name no folder there has is a new folder, made as it is saved in.
    let mut p = Picker::folder(docs.clone());
    key(&mut p, "newproj");
    assert_eq!(p.current(), Some(&Entry::Here));
    assert_eq!(key(&mut p, "<enter>"), Picked::Folder(docs.join("newproj")));
    if cfg!(unix) {
        let mut p = Picker::folder(dir.path().to_path_buf());
        key(&mut p, &format!("{}/newer", docs.display()));
        assert_eq!(key(&mut p, "<enter>"), Picked::Folder(docs.join("newer")));
    }
}

#[cfg(unix)]
#[test]
fn a_folder_that_could_not_be_saved_in_is_not_begun_in_next() {
    use std::os::unix::fs::PermissionsExt;
    let dir = tempfile::tempdir().unwrap();
    let shut = dir.path().join("shut");
    std::fs::create_dir(&shut).unwrap();
    std::fs::set_permissions(&shut, std::fs::Permissions::from_mode(0o555)).unwrap();
    let mut app = demo();
    app.save_dir = Some(dir.path().to_path_buf());
    let turing = app
        .shown
        .iter()
        .position(|m| m.from == "Alan Turing")
        .unwrap();
    app.list.select(Some(turing));
    press(
        &mut app,
        &["<enter>", "o", "s", "shut", "<enter>", "<enter>", "<enter>"],
    );
    let note = app.dialog.files().and_then(|f| f.note.clone());
    assert!(
        note.as_deref()
            .is_some_and(|n| n.starts_with("cannot save")),
        "{note:?}"
    );
    assert_eq!(app.save_dir.as_deref(), Some(dir.path()));
}

// ---- Pictures from elsewhere, fetched when asked ----

#[cfg(feature = "pages")]
#[test]
fn pictures_from_elsewhere_are_put_in_the_page_as_fetched() {
    let html = "<p>Hi<img alt=x SRC=\"https://cdn.example/a.png?w=1&amp;h=2\">\
                <img src='http://cdn.example/b.gif'><img src=\"cid:c1\">\
                <img src=\"https://cdn.example/gone.png\"></p>";
    let mut asked = Vec::new();
    let out = crate::app::with_remote_images(html, |url| {
        asked.push(url.to_string());
        (!url.contains("gone")).then(|| ("image/png".to_string(), b"PNG".to_vec()))
    });
    // Each asked for once, as its address reads, the message's own not.
    assert_eq!(
        asked,
        [
            "https://cdn.example/a.png?w=1&h=2",
            "http://cdn.example/b.gif",
            "https://cdn.example/gone.png"
        ]
    );
    assert_eq!(
        out.matches("data:image/png;base64,UE5H").count(),
        2,
        "{out}"
    );
    assert!(out.contains("cid:c1"), "{out}");
    // One not fetched is left as it was.
    assert!(out.contains("https://cdn.example/gone.png"), "{out}");
    assert!(!out.contains("cdn.example/a.png"), "{out}");
}
