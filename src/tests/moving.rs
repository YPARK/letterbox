//! A message moved to another account's folder: copied there first, found
//! there, and only then its original put in its own account's Trash.

use super::*;

/// The demo as two accounts, Demo and Work, each its own mail, and a hand
/// on each to see where a message is.
fn two_demos() -> (App, Demo, Demo) {
    let (home, work) = (Demo::new(), Demo::new().called("work"));
    let (seen_home, seen_work) = (home.handle(), work.handle());
    let mut app = App::new(
        vec![
            ("Demo".to_string(), Box::new(home) as Box<dyn Store>),
            ("Work".to_string(), Box::new(work) as Box<dyn Store>),
        ],
        PathBuf::from("/nowhere/accounts.toml"),
    );
    app.ledger = scratch_ledger();
    app.settle(Duration::from_secs(5));
    (app, seen_home, seen_work)
}

/// Pick "Notes on the engine" in the list, every Inbox together listing
/// it as one account's or the other's: its Message-ID, and that account.
fn pick_notes(app: &mut App) -> (String, usize) {
    let at = app
        .shown
        .iter()
        .position(|l| l.summary.subject.starts_with("Notes on the engine"))
        .unwrap();
    app.list.select(Some(at));
    (
        app.shown[at].summary.message_id.clone(),
        app.shown[at].account,
    )
}

/// In the labels dropped down, the one that is `folder` on account `a`.
fn choose_elsewhere(app: &mut App, a: usize, folder: &str) {
    let at = app
        .menu_items()
        .iter()
        .position(|p| matches!(p, app::Pick::Elsewhere(x, f, _) if *x == a && f == folder))
        .unwrap_or_else(|| panic!("no {folder} on account {a}: {:?}", app.menu_items()));
    for _ in 0..at {
        press(app, &["<down>"]);
    }
    press(app, &["<enter>"]);
    app.settle(Duration::from_secs(5));
}

#[test]
fn a_message_moved_to_another_accounts_folder_is_copied_first_then_trashed() {
    let (mut app, home, work) = two_demos();
    let (id, from) = pick_notes(&mut app);
    let to = 1 - from;
    let mut stores = [home, work];
    let names = ["Demo", "Work"];
    // m: another account's folder offered, as "Grant · Demo".
    press(&mut app, &["m", "grant"]);
    let screen = app::screen(&mut app, 150, 40);
    assert!(
        screen.contains(&format!("Grant · {}", names[to])),
        "{screen}"
    );
    choose_elsewhere(&mut app, to, "Projects/Grant");
    // Copied there; and only then its original out of its Inbox, to its own
    // account's Trash, to be put back from there.
    assert!(stores[to].holds("Projects/Grant", &id).unwrap(), "copied");
    assert!(
        !stores[from].holds("INBOX", &id).unwrap(),
        "out of the Inbox"
    );
    assert!(stores[from].holds("Trash", &id).unwrap(), "in its Trash");
    assert!(app.status.contains("✓ confirmed"), "{}", app.status);
}

#[test]
fn a_copy_refused_leaves_the_original_where_it_was() {
    let (mut app, home, work) = two_demos();
    let (id, from) = pick_notes(&mut app);
    let to = 1 - from;
    let mut stores = [home, work];
    let subject = app.current().unwrap().summary.subject.clone();
    stores[to].refuse(&subject);
    press(&mut app, &["m", "grant"]);
    choose_elsewhere(&mut app, to, "Projects/Grant");
    assert!(!stores[to].holds("Projects/Grant", &id).unwrap());
    assert!(
        stores[from].holds("INBOX", &id).unwrap(),
        "left in the Inbox"
    );
    assert!(!stores[from].holds("Trash", &id).unwrap());
    // Said, as what failed is.
    let screen = app::screen(&mut app, 150, 40);
    assert!(screen.contains("the server said no"), "{screen}");
}

#[test]
fn in_the_inbox_looked_through_a_message_may_go_to_another_account() {
    let (mut app, mut home, mut work) = two_demos();
    press(&mut app, &["<m-g>", "i"]);
    let s = app.map.snapshot.clone().unwrap();
    let rows = app.review_rows();
    let at = rows
        .iter()
        .position(|&i| {
            s.dots[i].subject.starts_with("Notes on the engine") && s.dots[i].key.account == "demo"
        })
        .unwrap();
    let id = s.dots[rows[at]].message_id.clone();
    for _ in 0..at {
        press(&mut app, &["j"]);
    }
    press(&mut app, &["m", "grant"]);
    choose_elsewhere(&mut app, 1, "Projects/Grant");
    assert_eq!(
        app.choice(rows[at]),
        app::Choice::Bring(1, "Projects/Grant".to_string())
    );
    assert!(!work.holds("Projects/Grant", &id).unwrap(), "not till $");
    press(&mut app, &["$", "y"]);
    app.settle(Duration::from_secs(5));
    assert!(work.holds("Projects/Grant", &id).unwrap());
    assert!(home.holds("Trash", &id).unwrap());
}
