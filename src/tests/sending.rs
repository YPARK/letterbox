//! A message sent is held a while first, to be taken back; and what is
//! easily got wrong is asked about once: no subject, a file spoken of and
//! none attached, many it goes to.
use super::*;

/// A new message to Ada, its subject and what is written given, sent.
fn write_and_send(app: &mut App, to: &str, subject: &str, body: &str) {
    press(
        app,
        &[
            "c", to, "<c-n>", "<c-n>", subject, "<c-n>", "<c-n>", body, "<c-c>", "<c-c>",
        ],
    );
}

/// Whether the demo's Sent holds a message with `subject`; the message
/// being written let go first, if it is still open.
fn sent(app: &mut App, subject: &str) -> bool {
    if app.compose.is_some() {
        press(app, &["<c-c>", "<c-k>", "y"]);
    }
    press(app, &["<esc>"]);
    let back = app.row;
    while app.focus != app::Focus::Side {
        press(app, &["h"]);
    }
    app.row = app
        .rows
        .iter()
        .position(
            |r| matches!(r, app::Row::Folder(0, i) if app.accounts[0].folders[*i].path == "Sent"),
        )
        .expect("Sent");
    press(app, &["<enter>"]);
    let there = app.shown.iter().any(|l| l.summary.subject == subject);
    app.row = back;
    there
}

#[test]
fn a_message_sent_is_held_and_c_g_takes_it_back() {
    let mut app = demo();
    app.send_hold = Duration::from_secs(10);
    write_and_send(&mut app, "ada@analytical.example", "Held", "Hi");
    assert!(app.compose.is_some(), "still open while held");
    assert!(app.status.contains("sending in"), "{}", app.status);
    assert!(app.status.contains("C-g"), "{}", app.status);
    // C-g: taken back, still there to change.
    press(&mut app, &["<c-g>"]);
    assert!(app.status.contains("not sent"), "{}", app.status);
    let compose = app.compose.as_ref().expect("kept");
    assert!(compose.buffer.text().contains("Held"));
    assert!(!sent(&mut app, "Held"));
}

#[test]
fn c_c_c_c_again_while_held_sends_it_at_once() {
    let mut app = demo();
    app.send_hold = Duration::from_secs(10);
    write_and_send(&mut app, "ada@analytical.example", "Now", "Hi");
    press(&mut app, &["<c-c>", "<c-c>"]);
    assert!(app.compose.is_none(), "{}", app.status);
    assert!(sent(&mut app, "Now"));
}

#[test]
fn a_message_held_is_sent_once_its_time_is_up() {
    let mut app = demo();
    app.send_hold = Duration::from_millis(30);
    write_and_send(&mut app, "ada@analytical.example", "Later", "Hi");
    assert!(app.compose.is_some());
    std::thread::sleep(Duration::from_millis(60));
    app.tick();
    app.settle(PATIENCE);
    assert!(app.compose.is_none(), "{}", app.status);
    assert!(sent(&mut app, "Later"));
}

#[test]
fn no_subject_is_asked_about_once() {
    let mut app = demo();
    write_and_send(&mut app, "ada@analytical.example", "", "Hi");
    assert!(app.compose.is_some(), "asked first");
    assert!(app.status.contains("no subject"), "{}", app.status);
    press(&mut app, &["<c-c>", "<c-c>"]);
    assert!(
        app.compose.is_none(),
        "sent the second time: {}",
        app.status
    );
}

#[test]
fn a_file_spoken_of_and_none_attached_is_asked_about() {
    let mut app = demo();
    write_and_send(
        &mut app,
        "ada@analytical.example",
        "Draft",
        "The draft is attached.",
    );
    assert!(app.compose.is_some(), "asked first");
    assert!(app.status.contains("attached"), "{}", app.status);
    press(&mut app, &["<c-c>", "<c-c>"]);
    assert!(app.compose.is_none(), "{}", app.status);
}

#[test]
fn a_file_spoken_of_only_in_what_is_quoted_is_not_asked_about() {
    let mut app = demo();
    write_and_send(
        &mut app,
        "ada@analytical.example",
        "Thanks",
        "Thanks!\n> The draft is attached.",
    );
    assert!(app.compose.is_none(), "{}", app.status);
}

#[test]
fn many_it_goes_to_is_asked_about() {
    let mut app = demo();
    let to: Vec<String> = (1..=12).map(|i| format!("p{i}@lab.example")).collect();
    write_and_send(&mut app, &to.join(", "), "All", "Hi");
    assert!(app.compose.is_some(), "asked first");
    assert!(app.status.contains("12"), "{}", app.status);
    press(&mut app, &["<c-c>", "<c-c>"]);
    assert!(app.compose.is_none(), "{}", app.status);
}

/// A server on this machine that takes mail, and files a copy in its own Sent
/// a moment after, as Exchange does: the account on it, and its mail.
fn filing_its_own_copy() -> (Account, Boxes) {
    let (port, log) = smtp_server();
    let (mut account, boxes) = server();
    account.smtp_host = Some("127.0.0.1".into());
    account.smtp_port = Some(port);
    let held = boxes.clone();
    std::thread::spawn(move || {
        let until = std::time::Instant::now() + PATIENCE;
        while log.lock().unwrap().data.is_empty() && std::time::Instant::now() < until {
            std::thread::sleep(Duration::from_millis(10));
        }
        std::thread::sleep(Duration::from_millis(300));
        let data = log.lock().unwrap().data.clone();
        let mut held = held.lock().unwrap();
        let sent = held.iter_mut().find(|b| b.name == "Sent Items").unwrap();
        sent.push(Letter::new(700, &["\\Seen"], data));
    });
    (account, boxes)
}

const A_LETTER: &[u8] = b"From: me@example.com\r\nTo: ada@example.com\r\nSubject: hi\r\n\
    Date: Tue, 20 Oct 2026 10:00:00 +0000\r\nMessage-ID: <once@example>\r\n\
    MIME-Version: 1.0\r\nContent-Type: text/plain; charset=utf-8\r\n\r\nHello\r\n";

#[test]
fn what_a_server_files_in_sent_itself_is_not_kept_there_twice() {
    let (account, boxes) = filing_its_own_copy();
    let mut imap = Imap::new(account);
    assert_eq!(imap.send(A_LETTER).unwrap(), "sent");
    let boxes = boxes.lock().unwrap();
    let sent = boxes.iter().find(|b| b.name == "Sent Items").unwrap();
    assert_eq!(sent.letters.len(), 1, "one copy, the server's");
    assert_eq!(sent.letters[0].uid, 700);
}

#[test]
fn a_letter_with_no_message_id_is_kept_as_it_cannot_be_looked_for() {
    let (port, _log) = smtp_server();
    let (mut account, boxes) = server();
    account.smtp_host = Some("127.0.0.1".into());
    account.smtp_port = Some(port);
    let mut imap = Imap::new(account);
    let raw = b"From: me@example.com\r\nTo: ada@example.com\r\nSubject: hi\r\n\r\nHello\r\n";
    let began = std::time::Instant::now();
    assert_eq!(imap.send(raw).unwrap(), "sent");
    assert!(
        began.elapsed() < Duration::from_secs(1),
        "no waiting for it"
    );
    let boxes = boxes.lock().unwrap();
    let sent = boxes.iter().find(|b| b.name == "Sent Items").unwrap();
    assert_eq!(sent.letters.len(), 1);
}

#[test]
fn how_long_sent_is_looked_in_goes_by_what_the_server_has_done() {
    use crate::imap::sent_looks;
    // Not known yet, the usual look.
    assert_eq!(sent_looks(false, 0), sent_looks(false, 2));
    // Known to file its own, longer, as it has been seen to do so.
    assert!(sent_looks(true, 0) > sent_looks(false, 0));
    // Seen not to, again and again: one look, and no waiting.
    assert_eq!(sent_looks(false, 3), 1);
    // Having filed its own, a miss or two does not stop the looking.
    assert_eq!(sent_looks(true, 9), sent_looks(true, 0));
}
