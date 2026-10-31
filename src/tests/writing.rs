//! Every account's Inbox together; replying, forwarding and renaming; and
//! the message being written, edited as Emacs edits.

use super::*;

// ---- every account's Inbox, Sent and Drafts together ----

/// The demo twice, as the accounts Home and Work; Work's inbox has a
/// message newer than any of Home's.
pub(super) fn two_accounts() -> App {
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
pub(super) fn list_row(app: &mut App, row: app::Row) {
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
pub(super) fn to_several() -> mail::Message {
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
    press(&mut app, &["U"]);
    assert!(app.status.contains("new mail"), "{}", app.status);
    // As in Gmail: r replies, a to all, f forwards; C-r too.
    for (keys, subject) in [
        (["r"], "Subject: Re: "),
        (["<c-r>"], "Subject: Re: "),
        (["a"], "Subject: Re: "),
        (["f"], "Subject: Fwd: "),
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
    for key in ["C-r", "C-n", "R"] {
        assert!(help.contains(key), "{key} in {help}");
    }
    // f alone forwards: C-f moves, as Emacs has it.
    assert!(help.contains("│ f "), "f in {help}");
    assert!(!help.contains("f C-f"), "{help}");
}

#[test]
fn c_g_leaves_the_message_asking_once_it_is_changed_as_c_c_c_k_does() {
    // Unchanged, C-g, or esc, leaves it at once, from its To line.
    for key in ["<c-g>", "<esc>"] {
        let mut app = demo();
        press(&mut app, &["c", key]);
        assert!(app.compose.is_none(), "{key}: {}", app.status);
    }
    // Changed, esc asks as C-g does.
    let mut app = demo();
    press(&mut app, &["c", "ada@analytical.example", "<esc>"]);
    assert!(app.compose.as_ref().unwrap().asking);
    // Changed, it asks; anything but y goes on writing.
    let mut app = demo();
    press(&mut app, &["c", "ada@analytical.example", "<c-g>"]);
    assert!(app.compose.as_ref().unwrap().asking);
    press(&mut app, &["n"]);
    assert!(app.compose.is_some());
    // The mark set, C-g lets it go first, as Emacs's does.
    press(&mut app, &["<c- >", "<c-g>"]);
    assert!(!app.compose.as_ref().unwrap().asking);
    press(&mut app, &["<c-g>", "y"]);
    assert!(app.compose.is_none());
    // A forward, its files carried, unchanged: asked too.
    let mut app = demo();
    let at = app
        .shown
        .iter()
        .position(|l| l.summary.from == "Alan Turing")
        .unwrap();
    app.list.select(Some(at));
    press(&mut app, &["<enter>", "f"]);
    assert!(!app.compose.as_ref().unwrap().files.is_empty());
    press(&mut app, &["<c-g>"]);
    assert!(app.compose.as_ref().unwrap().asking, "{}", app.status);
    let mut app = demo();
    press(&mut app, &["c", "ada@analytical.example"]);
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
    press(&mut app, &["M", ","]);
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
    press(&mut app, &["f", "ada@analytical.example", "<c-c>", "<c-c>"]);
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
    let papers = dir.path().join("papers");
    for name in ["figure.png", "notes.pdf"] {
        let line = format!("{}\n", papers.join(name).display());
        assert!(text.contains(&line), "{line} in {text}");
    }
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
    // No subject: asked once, and sent the second time.
    press(&mut app, &["<c-c>", "<c-c>"]);
    assert!(app.status.contains("no subject"), "{}", app.status);
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
    press(&mut app, &["f"]);
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
fn a_dropped_path_is_read_as_the_terminal_of_each_system_writes_it() {
    // Elsewhere a backslash escapes what follows it, as a space.
    assert_eq!(
        crate::app::dropped_words("/tmp/a\\ draft.txt '/tmp/plain.txt'", false),
        ["/tmp/a draft.txt", "/tmp/plain.txt"]
    );
    // On Windows it is what folders are parted by: kept, and a path with a
    // space is quoted.
    assert_eq!(
        crate::app::dropped_words(r#"C:\Users\a\plain.txt "C:\Users\a\a draft.txt""#, true),
        [r"C:\Users\a\plain.txt", r"C:\Users\a\a draft.txt"]
    );
    // An address: its path, the drive's after the root's slash.
    assert_eq!(
        crate::app::file_address("/Users/a/a%20draft.txt", false),
        "/Users/a/a draft.txt"
    );
    // On Windows, its folders parted as Windows parts them, so that it is
    // the same path as the file dropped.
    assert_eq!(
        crate::app::file_address("/C:/Users/a/a%20draft.txt", true),
        r"C:\Users\a\a draft.txt"
    );
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
    // As terminals paste dropped files: escaped, quoted, or as addresses;
    // on Windows, a path with a space quoted.
    let spaced_said = if cfg!(windows) {
        format!("\"{}\"", spaced.display())
    } else {
        spaced.display().to_string().replace(' ', "\\ ")
    };
    app.paste(&format!("{spaced_said} '{}'", plain.display()));
    let text = app.compose.as_ref().unwrap().buffer.text();
    assert!(text.contains("a draft.txt\n"), "{text}");
    assert!(text.contains("plain.txt\n"), "{text}");
    let address = format!(
        "file://{}{}",
        if cfg!(windows) { "/" } else { "" },
        spaced
            .display()
            .to_string()
            .replace(' ', "%20")
            .replace('\\', "/")
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
fn a_reply_to_a_name_with_a_comma_is_to_one_address() {
    let raw = b"From: \"Yan, Joseph\" <joseph.yan@emory.example>\r\n\
        To: you@example.com\r\nCc: \"Lee, \\\"Jo, Ann\\\"\" <ann@example.com>\r\n\
        Subject: Plans\r\n\r\nHi\r\n"
        .to_vec();
    let message = mail::message(1, raw).unwrap();
    let (text, _) = crate::compose::reply_all(&message, "you@example.com");
    assert!(
        text.starts_with(
            "To: \"Yan, Joseph\" <joseph.yan@emory.example>\n\
             Cc: \"Lee, \\\"Jo, Ann\\\"\" <ann@example.com>\n"
        ),
        "{text}"
    );
    let draft = crate::compose::parse(&text).unwrap();
    assert_eq!(draft.to.len(), 1, "{:?}", draft.to);
    assert_eq!(draft.cc.len(), 1, "{:?}", draft.cc);
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
