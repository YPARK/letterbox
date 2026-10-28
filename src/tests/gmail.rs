//! Gmail as letterbox is used on it: the whole of it, on a server that
//! does as Gmail does, filing from the list of its Inbox.

use super::*;

/// Letterbox on one Gmail account, settled: its Inbox listed. Without
/// All Mail, as it is when Gmail is told not to show it.
fn gmail_app(all_mail: bool) -> (App, Boxes) {
    gmail_app_with(all_mail, true)
}

/// As `gmail_app`, the Aims page in the Inbox, or in its label alone.
fn gmail_app_with(all_mail: bool, aims_in_inbox: bool) -> (App, Boxes) {
    let boxes = gmail_boxes();
    {
        let mut fake = boxes.lock().unwrap();
        // The label's folder holds what has the label, as Gmail's does.
        if !aims_in_inbox {
            fake[0].letters.retain(|l| l.msgid != 9021);
            for b in fake.iter_mut() {
                for l in b.letters.iter_mut().filter(|l| l.msgid == 9021) {
                    l.labels.retain(|x| x != "\\Inbox");
                }
            }
        }
        let aims = fake[2].letters[0].clone();
        let grant = fake
            .iter_mut()
            .find(|b| b.name == "Projects/Grant")
            .unwrap();
        grant.push(Letter { uid: 1, ..aims });
        if !all_mail {
            fake.retain(|b| b.name != "[Gmail]/All Mail");
        }
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

/// List the folder `path` of the one account, from the sidebar.
fn list_folder(app: &mut App, path: &str) {
    app.focus = app::Focus::Side;
    app.row = app
        .rows
        .iter()
        .position(|r| match *r {
            app::Row::Folder(a, i) => app.accounts[a].folders[i].path == path,
            _ => false,
        })
        .unwrap_or_else(|| panic!("no {path} in the sidebar"));
    press(app, &["<enter>"]);
    app.settle(Duration::from_secs(5));
    app.focus = app::Focus::List;
}

fn filed_from_a_label(all_mail: bool, aims_in_inbox: bool) {
    let (mut app, boxes) = gmail_app_with(all_mail, aims_in_inbox);
    list_folder(&mut app, "Projects/Grant");
    pick(&mut app, "Aims page, third pass");
    press(&mut app, &["m"]);
    press(&mut app, &["Family", "<enter>"]);
    // Moved from the label listed to the one given, and out of the Inbox.
    let (labels, inbox) = on_server(&boxes, "Aims page, third pass");
    assert_eq!(labels, ["Family"]);
    assert!(!inbox, "out of the Inbox");
    assert!(app.status.contains("✓ confirmed"), "{}", app.status);
}

#[test]
fn filed_from_a_gmail_labels_list_it_loses_that_label() {
    filed_from_a_label(true, true);
}

#[test]
fn without_all_mail_filed_from_a_gmail_labels_list_it_loses_that_label_too() {
    filed_from_a_label(false, true);
}

#[test]
fn in_a_label_alone_filed_from_its_list_it_loses_that_label() {
    // Gmail takes no label off in its own folder: it is found in the
    // folder of the label given.
    filed_from_a_label(false, false);
    filed_from_a_label(true, false);
}

#[test]
fn removing_a_gmail_label_takes_it_off_by_its_whole_name_and_c_x_u_gives_it_back() {
    let (mut app, boxes) = gmail_app(true);
    press(&mut app, &["<m-g>"]);
    app.settle(Duration::from_secs(5));
    let (labels, _) = on_server(&boxes, "Aims page, third pass");
    assert!(labels.contains(&"Projects/Grant".to_string()), "{labels:?}");
    app.ask_remove_label("Grant");
    assert!(
        matches!(app.dialog, app::Dialog::RemoveLabel(_)),
        "{}",
        app.status
    );
    press(&mut app, &["y"]);
    app.settle(Duration::from_secs(5));
    let (labels, _) = on_server(&boxes, "Aims page, third pass");
    assert!(
        !labels.contains(&"Projects/Grant".to_string()),
        "{labels:?}: {}",
        app.status
    );
    press(&mut app, &["<c-x>", "u"]);
    app.settle(Duration::from_secs(5));
    let (labels, _) = on_server(&boxes, "Aims page, third pass");
    assert!(
        labels.contains(&"Projects/Grant".to_string()),
        "{labels:?}: {}",
        app.status
    );
}

#[test]
fn archived_or_filed_on_gmail_c_x_u_gives_its_labels_back() {
    let (mut app, boxes) = gmail_app(true);
    let was = on_server(&boxes, "R01 resubmission");
    assert!(was.1, "in the Inbox");
    // Archived: out of the Inbox, and back by C-x u.
    pick(&mut app, "R01 resubmission");
    press(&mut app, &["e"]);
    app.settle(Duration::from_secs(5));
    assert!(!on_server(&boxes, "R01 resubmission").1, "{}", app.status);
    press(&mut app, &["<c-x>", "u"]);
    app.settle(Duration::from_secs(5));
    assert_eq!(on_server(&boxes, "R01 resubmission"), was, "{}", app.status);
    // Filed under a label: the label off again, the Inbox's back.
    app.settle(Duration::from_secs(5));
    pick(&mut app, "R01 resubmission");
    press(&mut app, &["m"]);
    press(&mut app, &["Family", "<enter>"]);
    app.settle(Duration::from_secs(5));
    assert_ne!(on_server(&boxes, "R01 resubmission"), was);
    press(&mut app, &["<c-x>", "u"]);
    app.settle(Duration::from_secs(5));
    assert_eq!(on_server(&boxes, "R01 resubmission"), was, "{}", app.status);
}
