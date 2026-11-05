//! The sidebar folded: an account to its Inbox, a folder to itself, its
//! subfolders hidden; opened again as it was; and remembered.

use super::*;

/// The sidebar's rows of the first account: its header, then each folder
/// shown, by name.
fn side(app: &App) -> Vec<String> {
    app.rows
        .iter()
        .filter_map(|r| match *r {
            app::Row::Account(0) => Some("ACCOUNT".to_string()),
            app::Row::Folder(0, i) => Some(app.accounts[0].folders[i].name.clone()),
            _ => None,
        })
        .collect()
}

/// Pick the folder `name` in the sidebar.
fn pick(app: &mut App, name: &str) {
    app.focus = Focus::Side;
    app.row = app
        .rows
        .iter()
        .position(
            |r| matches!(*r, app::Row::Folder(0, i) if app.accounts[0].folders[i].name == name),
        )
        .unwrap();
}

fn picked(app: &App) -> String {
    match app.rows[app.row] {
        app::Row::Folder(0, i) => app.accounts[0].folders[i].name.clone(),
        app::Row::Account(_) => "ACCOUNT".to_string(),
        _ => String::new(),
    }
}

const ALL: [&str; 9] = [
    "ACCOUNT", "Inbox", "Drafts", "Sent", "Archive", "Junk", "Trash", "Projects", "Grant",
];

#[test]
fn space_folds_an_account_to_its_inbox_and_opens_it_again() {
    let mut app = demo();
    assert_eq!(side(&app), ALL);
    pick(&mut app, "Sent");
    press(&mut app, &["<space>"]);
    assert_eq!(side(&app), ["ACCOUNT", "Inbox"]);
    assert_eq!(
        picked(&app),
        "Inbox",
        "the one picked hidden, its Inbox picked"
    );
    let screen = app::screen(&mut app, 120, 40);
    assert!(screen.contains("▸ DEMO"), "{screen}");
    assert!(screen.contains("7 more"), "{screen}");
    press(&mut app, &["<space>"]);
    assert_eq!(side(&app), ALL);
    let screen = app::screen(&mut app, 120, 40);
    assert!(screen.contains("▾ DEMO"), "{screen}");
}

#[test]
fn space_folds_a_folder_to_itself_its_subfolders_hidden() {
    let mut app = demo();
    pick(&mut app, "Projects");
    press(&mut app, &["<space>"]);
    assert_eq!(side(&app), &ALL[..8]);
    assert_eq!(picked(&app), "Projects");
    let screen = app::screen(&mut app, 120, 40);
    assert!(screen.contains("Projects +1"), "{screen}");
    // A folder with none under it folds its account.
    pick(&mut app, "Junk");
    press(&mut app, &["<space>", "<space>"]);
    assert_eq!(side(&app), &ALL[..8], "Projects still folded within");
    pick(&mut app, "Projects");
    press(&mut app, &["<space>"]);
    assert_eq!(side(&app), ALL);
    // j and k go along what is shown.
    pick(&mut app, "Projects");
    press(&mut app, &["<space>", "k", "j"]);
    assert_eq!(picked(&app), "Projects");
}

#[test]
fn a_click_on_an_account_folds_it_and_on_it_again_opens_it() {
    let mut app = demo();
    let _ = app::screen(&mut app, 120, 40);
    let header = app
        .rows
        .iter()
        .position(|r| matches!(r, app::Row::Account(0)))
        .unwrap();
    click_at(&mut app, app::Target::Row(header));
    assert_eq!(side(&app), ["ACCOUNT", "Inbox"]);
    let _ = app::screen(&mut app, 120, 40);
    click_at(&mut app, app::Target::Row(header));
    assert_eq!(side(&app), ALL);
}

#[test]
fn what_is_folded_is_remembered_as_letterbox_next_opens() {
    let dir = tempfile::tempdir().unwrap();
    let look = dir.path().join("look");
    let mut app = demo();
    app.look_file = Some(look.clone());
    pick(&mut app, "Projects");
    press(&mut app, &["<space>"]);
    pick(&mut app, "Trash");
    press(&mut app, &["<space>"]);
    let said = std::fs::read_to_string(&look).unwrap();
    assert!(
        said.contains("fold Demo\t\n") && said.contains("fold Demo\tProjects\n"),
        "{said:?}"
    );

    let mut again = demo();
    again.look_file = Some(look);
    again.recall_look();
    assert_eq!(side(&again), ["ACCOUNT", "Inbox"]);
    pick(&mut again, "Inbox");
    press(&mut again, &["<space>"]);
    assert_eq!(side(&again), &ALL[..8], "Projects as it was left");
}

#[test]
fn a_click_on_all_accounts_folds_every_one_and_again_opens_them() {
    let mut app = two_accounts();
    let _ = app::screen(&mut app, 120, 40);
    let everyone = app
        .rows
        .iter()
        .position(|r| matches!(r, app::Row::Everyone))
        .unwrap();
    let shown = |app: &App| {
        app.rows
            .iter()
            .filter(|r| matches!(r, app::Row::Folder(..)))
            .count()
    };
    let before = shown(&app);
    click_at(&mut app, app::Target::Row(everyone));
    assert_eq!(shown(&app), 2, "each account's Inbox");
    let screen = app::screen(&mut app, 120, 40);
    assert!(screen.matches("▸ ").count() >= 2, "{screen}");
    click_at(&mut app, app::Target::Row(everyone));
    assert_eq!(shown(&app), before);
}

/// The demo, with folders inside its Inbox and its Archive, as some servers
/// keep them.
fn nested() -> App {
    let mut demo = Demo::new();
    for path in ["INBOX/Work", "INBOX/Lists", "Archive/2020"] {
        demo.create(path).unwrap();
    }
    let mut app = App::new(
        vec![("Demo".to_string(), Box::new(demo) as Box<dyn Store>)],
        PathBuf::from("/nowhere/accounts.toml"),
    );
    app.ledger = scratch_ledger();
    app.settle(PATIENCE);
    app
}

#[test]
fn an_account_folded_with_folders_in_its_inbox_opens_again_by_space() {
    let mut app = nested();
    let all = side(&app);
    pick(&mut app, "Sent");
    press(&mut app, &["<space>"]);
    assert_eq!(side(&app), ["ACCOUNT", "Inbox"]);
    press(&mut app, &["<space>"]);
    assert_eq!(side(&app), all, "the account opened, not its Inbox folded");
}

#[test]
fn a_folder_holds_only_what_is_within_it_by_its_path() {
    let mut app = nested();
    let all = side(&app);
    pick(&mut app, "Archive");
    press(&mut app, &["<space>"]);
    let folded = side(&app);
    assert!(!folded.contains(&"2020".to_string()), "{folded:?}");
    assert_eq!(folded.len(), all.len() - 1, "only 2020 hidden: {folded:?}");
    let screen = app::screen(&mut app, 120, 40);
    assert!(screen.contains("Archive +1"), "{screen}");
}

#[test]
fn folding_another_account_leaves_the_one_picked_and_one_folded_away_opens_its_inbox() {
    let mut app = two_accounts();
    let _ = app::screen(&mut app, 120, 40);
    let sent = |app: &App, a: usize| {
        app.rows
            .iter()
            .position(|r| matches!(*r, app::Row::Folder(x, i) if x == a && app.accounts[a].folders[i].name == "Sent"))
            .unwrap()
    };
    app.focus = Focus::Side;
    app.row = sent(&app, 0);
    let header = app
        .rows
        .iter()
        .position(|r| matches!(r, app::Row::Account(1)))
        .unwrap();
    click_at(&mut app, app::Target::Row(header));
    assert_eq!(
        app.rows[app.row],
        app.rows[sent(&app, 0)],
        "left where it was"
    );
    // Folding its own account: its Inbox picked, and listed.
    app.focus = Focus::Side;
    app.row = sent(&app, 0);
    press(&mut app, &["<enter>"]);
    assert_eq!(app.folder_name(), "Sent");
    app.focus = Focus::Side;
    press(&mut app, &["<space>"]);
    std::thread::sleep(Duration::from_millis(600));
    app.tick();
    app.settle(PATIENCE);
    assert_eq!(app.folder_name(), "Inbox", "{}", app.status);
}

#[test]
fn what_is_folded_of_a_folder_or_account_no_more_is_let_go() {
    let dir = tempfile::tempdir().unwrap();
    let look = dir.path().join("look");
    std::fs::write(
        &look,
        "list\nfold Demo\tGone\nfold Demo\tProjects\nfold Elsewhere\tAny\nopen Demo\n",
    )
    .unwrap();
    let mut app = demo();
    app.look_file = Some(look);
    app.recall_look();
    let kept: Vec<(String, String)> = app
        .folded
        .iter()
        .flat_map(|(a, ps)| ps.iter().map(move |p| (a.clone(), p.clone())))
        .collect();
    assert_eq!(kept, [("Demo".to_string(), "Projects".to_string())]);
}

#[test]
fn a_folder_is_within_another_only_by_its_delimiter() {
    let within = |path, delimiter, of| Folder::new(path, delimiter, None, true).is_within(of);
    assert!(within("Foo.bar", Some("."), "Foo"));
    assert!(within("Foo/bar", None, "Foo"));
    assert!(!within("Foobar", None, "Foo"));
    assert!(!within("Foo", None, "Foo"));
    // A server that says its delimiter is nothing: not every name is under.
    assert!(!within("Foobar", Some(""), "Foo"));
}

/// Click on the first account's header, folding or opening it.
fn click_first_account(app: &mut App) {
    let _ = app::screen(app, 120, 40);
    let at = app
        .rows
        .iter()
        .position(|r| matches!(r, app::Row::Account(0)))
        .unwrap();
    click_at(app, app::Target::Row(at));
}

#[test]
fn of_several_accounts_each_opens_folded_until_it_is_opened_then_as_it_was_left() {
    let dir = tempfile::tempdir().unwrap();
    let look = dir.path().join("look");
    let folded = |app: &App| (app.is_folded(0, None), app.is_folded(1, None));
    // Never opened before.
    let mut app = two_accounts();
    app.look_file = Some(look.clone());
    app.recall_look();
    assert_eq!(folded(&app), (true, true));
    // Opened, it stays so.
    click_first_account(&mut app);
    assert_eq!(folded(&app), (false, true));
    let mut again = two_accounts();
    again.look_file = Some(look.clone());
    again.recall_look();
    assert_eq!(folded(&again), (false, true));
    // Folded again, it stays so too.
    click_first_account(&mut again);
    let mut last = two_accounts();
    last.look_file = Some(look);
    last.recall_look();
    assert_eq!(folded(&last), (true, true));
}

#[test]
fn one_account_alone_opens_open_as_its_folders_are_all_it_has() {
    let dir = tempfile::tempdir().unwrap();
    let look = dir.path().join("look");
    let mut app = demo();
    app.look_file = Some(look.clone());
    app.recall_look();
    assert_eq!(side(&app), ALL);
    // Folded by its user, it stays so.
    pick(&mut app, "Trash");
    press(&mut app, &["<space>"]);
    let mut again = demo();
    again.look_file = Some(look);
    again.recall_look();
    assert_eq!(side(&again), ["ACCOUNT", "Inbox"]);
}

#[test]
fn h_where_it_is_already_leaves_what_is_marked_marked() {
    let mut app = demo();
    pick(&mut app, "Inbox");
    press(&mut app, &["<enter>"]);
    app.focus = Focus::List;
    let _ = app::screen(&mut app, 120, 40);
    super::selecting::ctrl_click(&mut app, 0);
    assert!(app.map.region.is_some(), "{}", app.status);
    press(&mut app, &["H"]);
    assert!(app.map.region.is_some());
}

#[test]
fn h_goes_to_every_inbox_and_on_to_every_sent_and_every_draft() {
    let mut app = two_accounts();
    let _ = app::screen(&mut app, 120, 40);
    for role in [
        mail::Role::Inbox,
        mail::Role::Sent,
        mail::Role::Drafts,
        mail::Role::Inbox,
    ] {
        press(&mut app, &["H"]);
        assert_eq!(app.rows[app.row], app::Row::All(role));
        assert_eq!(
            app.folder_name(),
            format!("{}, all accounts", role.name()),
            "{}",
            app.status
        );
        assert_eq!(app.focus, Focus::List);
    }
    assert!(app::screen(&mut app, 200, 40).contains("H every Sent"));
}

#[test]
fn h_with_one_account_goes_to_its_inbox_and_stays_there_folded_or_not() {
    for folded in [false, true] {
        let mut app = demo();
        pick(&mut app, "Trash");
        press(&mut app, &["<enter>"]);
        if folded {
            pick(&mut app, "Trash");
            press(&mut app, &["<space>"]);
        }
        for _ in 0..3 {
            press(&mut app, &["H"]);
            assert_eq!(picked(&app), "Inbox");
            assert_eq!(app.folder_name(), "Inbox");
            assert!(app::screen(&mut app, 200, 40).contains("H Inbox"));
        }
    }
}

#[test]
fn h_from_the_map_goes_to_every_inbox_whatever_is_behind_it() {
    let mut app = two_accounts();
    let _ = app::screen(&mut app, 120, 40);
    press(&mut app, &["H", "H"]);
    assert_eq!(app.rows[app.row], app::Row::All(mail::Role::Sent));
    press(&mut app, &["M"]);
    assert_eq!(app.map.mode, crate::map::Mode::Map);
    assert_eq!(
        app.together_next(),
        mail::Role::Inbox,
        "not what is behind it"
    );
    press(&mut app, &["H"]);
    assert_eq!(app.map.mode, crate::map::Mode::List);
    assert_eq!(app.rows[app.row], app::Row::All(mail::Role::Inbox));
}
