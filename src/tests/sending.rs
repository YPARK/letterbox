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
    app.settle(Duration::from_secs(5));
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
