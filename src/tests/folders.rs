//! A label's folder split into subfolders, or removed, its mail filed
//! elsewhere: shown first, done on y, undone by C-x u; and a folder deleted
//! only when it is empty, with none under it.
use super::*;

#[test]
fn a_folder_is_deleted_only_empty_and_with_none_under_it() {
    let mut store = Demo::new();
    // Holding mail: never.
    let err = store.delete_folder("Archive").unwrap_err();
    assert!(err.to_string().contains("not empty"), "{err}");
    assert!(store.folders().unwrap().iter().any(|f| f.path == "Archive"));
    // With a folder under it, though empty itself: never.
    let err = store.delete_folder("Projects").unwrap_err();
    assert!(err.to_string().contains("under it"), "{err}");
    // Empty, none under it: deleted.
    store.create("Old").unwrap();
    store.delete_folder("Old").unwrap();
    assert!(!store.folders().unwrap().iter().any(|f| f.path == "Old"));
    // Not there: said.
    assert!(store.delete_folder("Old").is_err());
}

/// Whether account 0 has the folder `path`.
fn has_folder(app: &App, path: &str) -> bool {
    app.accounts[0].folders.iter().any(|f| f.path == path)
}

/// The labels of the map's message with `subject`.
fn labels_of(app: &App, subject: &str) -> Vec<String> {
    dot_labels(app, subject).0
}

#[test]
fn removing_a_label_files_its_mail_where_the_map_would_and_c_x_u_puts_it_back() {
    let mut app = super::mapscreen::demo_map();
    assert!(has_folder(&app, "Projects/Grant"));
    let before = labels_of(&app, "Letter of support, signed");
    assert!(before.contains(&"Grant".to_string()), "{before:?}");
    app.ask_remove_label("Grant");
    let app::Dialog::RemoveLabel(plan) = &app.dialog else {
        panic!("shown first: {}", app.status);
    };
    // Each where the map would file it, never to Grant; Trash nowhere.
    assert!(plan.to.iter().all(|(to, _)| to.as_deref() != Some("Grant")));
    let total: usize = plan.to.iter().map(|(_, k)| k.len()).sum();
    assert!(total > 0);
    let screen = app::screen(&mut app, 150, 45);
    assert!(screen.contains("Remove Grant"), "{screen}");
    assert!(screen.contains("No message is thrown away"), "{screen}");
    // Anything but y leaves it.
    press(&mut app, &["n"]);
    assert!(has_folder(&app, "Projects/Grant"));
    // y: filed elsewhere, and the folder deleted once empty.
    app.ask_remove_label("Grant");
    press(&mut app, &["y"]);
    app.settle(Duration::from_secs(5));
    assert!(!has_folder(&app, "Projects/Grant"), "{}", app.status);
    assert!(
        !labels_of(&app, "Letter of support, signed").contains(&"Grant".to_string()),
        "{}",
        app.status
    );
    assert!(app.status.contains("C-x u"), "{}", app.status);
    // C-x u: the folder made again, its mail moved back.
    press(&mut app, &["<c-x>", "u"]);
    app.settle(Duration::from_secs(5));
    assert!(has_folder(&app, "Projects/Grant"), "{}", app.status);
    assert!(
        labels_of(&app, "Letter of support, signed").contains(&"Grant".to_string()),
        "{}",
        app.status
    );
}

#[test]
fn a_label_with_folders_under_it_is_never_removed() {
    let mut app = super::mapscreen::demo_map();
    app.ask_remove_label("Projects");
    assert!(!app.dialog.is_open());
    assert!(app.status.contains("under it"), "{}", app.status);
}

#[test]
fn splitting_a_label_parts_it_by_what_its_mail_is_about_and_c_x_u_puts_it_back() {
    let mut app = super::mapscreen::demo_map();
    // A label letterbox keeps, on mail of two kinds: the grant's, and the
    // robots' and the travel desk's.
    let keys: Vec<crate::index::Key> = {
        let s = app.map.snapshot.as_ref().unwrap();
        let grant = super::labels_model::GRANT_MAIL;
        let mut keys: Vec<_> = s
            .dots
            .iter()
            .filter(|d| grant.contains(&d.subject.as_str()))
            .map(|d| d.key.clone())
            .collect();
        keys.extend(
            s.dots
                .iter()
                .filter(|d| {
                    [
                        "ci@runner.example",
                        "stay@hotel.example",
                        "travel@uni.example",
                    ]
                    .contains(&d.address.as_str())
                })
                .map(|d| d.key.clone()),
        );
        keys
    };
    assert!(keys.len() >= 12, "{}", keys.len());
    {
        let mut index = app.index.lock().unwrap();
        let events = keys
            .iter()
            .map(|k| crate::index::Kept::Label {
                key: k.clone(),
                label: "Heap".to_string(),
                on: true,
            })
            .collect();
        index.keep(events).unwrap();
    }
    app.settle(Duration::from_secs(5));
    app.ask_split_label("Heap");
    let app::Dialog::Split(plan) = &app.dialog else {
        panic!("shown first: {}", app.status);
    };
    assert!(plan.parts.len() >= 2, "{:?}", plan.parts);
    assert_eq!(
        plan.parts.iter().map(|p| p.keys.len()).sum::<usize>(),
        keys.len()
    );
    let first = plan.parts[0].keys[0].clone();
    // ⏎ types the first part's name over.
    press(&mut app, &["<enter>", "<c-u>", "Firstpart", "<enter>"]);
    let app::Dialog::Split(plan) = &app.dialog else {
        panic!("still shown");
    };
    assert_eq!(plan.parts[0].name, "Firstpart");
    let screen = app::screen(&mut app, 150, 45);
    assert!(screen.contains("Heap/Firstpart"), "{screen}");
    press(&mut app, &["y"]);
    app.settle(Duration::from_secs(5));
    let labels = |app: &App, k: &crate::index::Key| app.index.lock().unwrap().labels(k);
    let got = labels(&app, &first);
    assert!(
        got.contains(&"Firstpart".to_string()) && got.contains(&"Heap".to_string()),
        "{got:?}: {}",
        app.status
    );
    // C-x u: as it was.
    press(&mut app, &["<c-x>", "u"]);
    app.settle(Duration::from_secs(5));
    let got = labels(&app, &first);
    assert!(!got.contains(&"Firstpart".to_string()), "{got:?}");
    assert!(got.contains(&"Heap".to_string()), "{got:?}");
}

#[test]
fn a_label_too_small_is_not_split() {
    let mut app = super::mapscreen::demo_map();
    app.ask_split_label("Grant");
    assert!(!app.dialog.is_open());
    assert!(app.status.contains("too few"), "{}", app.status);
}
