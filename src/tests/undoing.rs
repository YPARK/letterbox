//! C-x u puts back what was last moved, however it was: archived, filed,
//! trashed alone, or what the Inbox looked through did; each where it was,
//! as its server keeps it, and what moving it taught taken back.
use super::filing::{dot_labels, down_subject, pick_subject};
use super::mapscreen::demo_map;
use super::*;

const AIMS: &str = "Aims page, third pass";

/// Whether the map has the message `subject` in the Inbox: not, if the
/// map has it nowhere, as in Trash.
fn in_inbox(app: &App, subject: &str) -> bool {
    let s = app.map.snapshot.as_ref().unwrap();
    s.dots
        .iter()
        .any(|d| d.subject == subject && d.system.inbox)
}

/// C-x u, and its servers' answers, and the map drawn again after.
fn undo(app: &mut App) {
    press(app, &["<c-x>", "u"]);
    app.settle(Duration::from_secs(5));
}

#[test]
fn one_archived_is_put_back_by_c_x_u() {
    let mut app = demo_map();
    assert!(in_inbox(&app, AIMS));
    down_subject(&mut app, AIMS);
    press(&mut app, &["e"]);
    assert!(!in_inbox(&app, AIMS), "archived: {}", app.status);
    undo(&mut app);
    assert!(in_inbox(&app, AIMS), "back: {}", app.status);
    assert!(app.status.contains("put back"), "{}", app.status);
}

#[test]
fn one_filed_is_put_back_by_c_x_u_its_label_taken_off() {
    let mut app = demo_map();
    down_subject(&mut app, AIMS);
    press(&mut app, &["m"]);
    pick_subject(&mut app, "Letter of support, signed");
    press(&mut app, &["<enter>"]);
    let (labels, inbox) = dot_labels(&app, AIMS);
    assert!(
        labels.contains(&"Grant".to_string()) && !inbox,
        "{labels:?}"
    );
    undo(&mut app);
    let (labels, inbox) = dot_labels(&app, AIMS);
    assert!(inbox, "back in the Inbox: {}", app.status);
    assert!(!labels.contains(&"Grant".to_string()), "{labels:?}");
}

#[test]
fn one_trashed_alone_is_put_back_and_what_it_taught_taken_back() {
    let mut app = demo_map();
    let thrown = |app: &App| app.index.lock().unwrap().filter.thrown;
    let before = thrown(&app);
    down_subject(&mut app, AIMS);
    press(&mut app, &["d"]);
    assert_eq!(thrown(&app), before + 1);
    undo(&mut app);
    assert!(in_inbox(&app, AIMS), "back: {}", app.status);
    assert_eq!(thrown(&app), before, "the lesson taken back");
}

#[test]
fn what_the_inbox_looked_through_did_is_put_back_together() {
    let mut app = demo_map();
    press(&mut app, &["i"]);
    let rows = app.review_rows();
    let s = app.map.snapshot.clone().unwrap();
    let subjects: Vec<String> = rows[..2]
        .iter()
        .map(|&i| s.dots[i].subject.clone())
        .collect();
    // The first discarded, the second filed under the label first
    // offered: each chosen alone.
    press(&mut app, &["d", "m", "<enter>"]);
    press(&mut app, &["$", "y"]);
    app.settle(Duration::from_secs(5));
    if matches!(app.dialog, app::Dialog::Applied(_)) {
        press(&mut app, &["x"]);
    }
    for subject in &subjects {
        assert!(!in_inbox(&app, subject), "{subject} done: {}", app.status);
    }
    press(&mut app, &["<esc>"]);
    undo(&mut app);
    for subject in &subjects {
        assert!(in_inbox(&app, subject), "{subject} back: {}", app.status);
    }
}

#[test]
fn c_x_u_before_the_server_has_answered_says_so_and_puts_it_back_after() {
    let mut app = demo_map();
    down_subject(&mut app, AIMS);
    // e, and C-x u at once, not waiting for the server.
    for key in ["e", "<c-x>", "u"].iter().flat_map(|k| app::keys(k)) {
        app.key(key);
    }
    assert!(app.status.contains("C-x u again"), "{}", app.status);
    app.settle(Duration::from_secs(5));
    undo(&mut app);
    assert!(in_inbox(&app, AIMS), "back: {}", app.status);
}
