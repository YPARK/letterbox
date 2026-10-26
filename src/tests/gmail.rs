//! Gmail as letterbox is used on it: the whole of it, on a server that
//! does as Gmail does, filing from the list of its Inbox.

use super::*;

/// Letterbox on one Gmail account, settled: its Inbox listed. Without
/// All Mail, as it is when Gmail is told not to show it.
fn gmail_app(all_mail: bool) -> (App, Boxes) {
    let boxes = gmail_boxes();
    if !all_mail {
        let mut fake = boxes.lock().unwrap();
        fake.retain(|b| b.name != "[Gmail]/All Mail");
    }
    let (account, boxes) = serve_boxes(boxes);
    let mut app = App::new(
        vec![(
            "Gmail".to_string(),
            Box::new(Imap::new(account)) as Box<dyn Store>,
        )],
        PathBuf::from("/nowhere/accounts.toml"),
    );
    app.ledger = scratch_ledger();
    app.settle(Duration::from_secs(5));
    (app, boxes)
}

fn pick(app: &mut App, subject: &str) {
    let at = app
        .shown
        .iter()
        .position(|l| l.summary.subject == subject)
        .unwrap_or_else(|| panic!("{subject} not listed: {:?}", app.status));
    app.list.select(Some(at));
}

/// The labels the server has on the message called `subject`, and whether
/// its Inbox holds it.
fn on_server(boxes: &Boxes, subject: &str) -> (Vec<String>, bool) {
    let boxes = boxes.lock().unwrap();
    let called = |l: &&Letter| String::from_utf8_lossy(&l.raw).contains(subject);
    let letter = boxes
        .iter()
        .flat_map(|b| b.letters.iter())
        .find(called)
        .unwrap_or_else(|| panic!("no {subject}"));
    let inbox = boxes.iter().find(|b| b.name == "INBOX").unwrap();
    let mut labels = letter.labels.clone();
    labels.sort();
    (
        labels,
        inbox.letters.iter().any(|l| l.msgid == letter.msgid),
    )
}

fn filed_from_the_inbox_list(all_mail: bool) {
    let (mut app, boxes) = gmail_app(all_mail);
    pick(&mut app, "R01 resubmission");
    press(&mut app, &["m"]);
    press(&mut app, &["Family", "<enter>"]);
    let (labels, inbox) = on_server(&boxes, "R01 resubmission");
    assert_eq!(labels, ["Family", "\\Important"]);
    assert!(!inbox, "out of the Inbox");
    assert!(app.status.contains("✓ confirmed"), "{}", app.status);
    assert!(
        app.shown
            .iter()
            .all(|l| l.summary.subject != "R01 resubmission")
    );
}

#[test]
fn filed_from_the_gmail_inbox_list_it_is_out_of_the_inbox() {
    filed_from_the_inbox_list(true);
}

#[test]
fn without_all_mail_filed_from_the_gmail_inbox_it_is_out_of_the_inbox_too() {
    filed_from_the_inbox_list(false);
}
