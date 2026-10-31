//! A message's files saved: picked from a dialog of their own, into a
//! folder picked, each under a name looked at first.

use super::*;

/// The demo's letter with two attachments, read.
fn read_turing(app: &mut App) {
    let turing = app
        .shown
        .iter()
        .position(|m| m.from == "Alan Turing")
        .unwrap();
    app.list.select(Some(turing));
    press(app, &["<enter>"]);
}

#[test]
fn a_name_is_begun_on_its_own_its_stem_picked_to_type_over() {
    use crate::app::Name;
    let mut n = Name::new("computable-numbers.pdf");
    assert_eq!(n.picked(), "computable-numbers");
    for k in app::keys("turing") {
        n.key(k);
    }
    assert_eq!(n.text, "turing.pdf");
    assert_eq!(n.picked(), "");

    // ← lets the stem go, the cursor at its start; typed there, put in.
    let mut n = Name::new("states.txt");
    for k in app::keys("<left>").into_iter().chain(app::keys("old-")) {
        n.key(k);
    }
    assert_eq!(n.text, "old-states.txt");
    // To its end, and the extension changed.
    for k in app::keys("<end>")
        .into_iter()
        .chain(app::keys("<backspace>"))
        .chain(app::keys("<backspace>"))
        .chain(app::keys("<backspace>"))
        .chain(app::keys("md"))
    {
        n.key(k);
    }
    assert_eq!(n.text, "old-states.md");

    // A name without an extension is picked whole; C-u clears it.
    let mut n = Name::new("README");
    assert_eq!(n.picked(), "README");
    n.key(app::keys("<c-u>")[0]);
    assert_eq!(n.text, "");
}

#[test]
fn s_saves_from_a_dialog_of_its_own_each_named_first() {
    let dir = tempfile::tempdir().unwrap();
    let mut app = demo();
    app.save_dir = Some(dir.path().to_path_buf());
    read_turing(&mut app);

    // Its attachments alone, the first picked.
    press(&mut app, &["s"]);
    let files = app.dialog.files().expect("listed to save");
    let names: Vec<String> = files
        .items
        .iter()
        .map(|i| match i {
            app::Item::File { name, .. } => name.clone(),
            other => panic!("not a file: {other:?}"),
        })
        .collect();
    assert_eq!(names, ["states.txt", "computable-numbers.pdf"]);
    let screen = app::screen(&mut app, 120, 40);
    assert!(
        screen.contains("Save") && screen.contains("a save all"),
        "{screen}"
    );

    // ⏎: the folder, then the name, its stem picked; typed over, saved so.
    press(&mut app, &["j", "<enter>", "<enter>"]);
    let name = app.dialog.naming().expect("asked its name");
    assert_eq!(name.text, "computable-numbers.pdf");
    assert_eq!(name.picked(), "computable-numbers");
    let screen = app::screen(&mut app, 120, 40);
    assert!(
        screen.contains("Save as") && screen.contains("computable-numbers.pdf"),
        "{screen}"
    );
    press(&mut app, &["turing1936", "<enter>"]);
    assert!(dir.path().join("turing1936.pdf").is_file());
    assert!(app.dialog.files().is_some(), "back to its files");

    // ⏎ on the name as it is keeps it.
    press(&mut app, &["<enter>", "<enter>", "<enter>"]);
    assert!(dir.path().join("computable-numbers.pdf").is_file());

    // Esc from the name goes back to the folders, and from them to the files.
    press(&mut app, &["<enter>", "<enter>", "<esc>"]);
    assert!(app.dialog.saving().is_some());
    press(&mut app, &["<esc>"]);
    assert!(app.dialog.files().is_some());

    // a saves them all, named as they are, and closes.
    press(&mut app, &["a", "<enter>"]);
    assert!(dir.path().join("states.txt").is_file());
    assert!(dir.path().join("computable-numbers (2).pdf").is_file());
    assert!(!app.dialog.is_open(), "{}", app.status);
}

#[test]
fn s_on_a_message_without_attachments_says_so() {
    let mut app = demo();
    let ada = app
        .shown
        .iter()
        .position(|m| m.from == "Ada Lovelace")
        .unwrap();
    app.list.select(Some(ada));
    press(&mut app, &["<enter>"]);
    assert!(app.message().unwrap().attachments.is_empty());
    for key in ["s", "S"] {
        press(&mut app, &[key]);
        assert!(!app.dialog.is_open());
        assert_eq!(app.status, "no attachments to save");
    }
}

#[test]
fn bang_flags_and_capital_s_saves_all_into_a_folder_picked() {
    let dir = tempfile::tempdir().unwrap();
    let mut app = demo();
    app.save_dir = Some(dir.path().to_path_buf());
    read_turing(&mut app);

    // ! flags, and again unflags.
    let flagged = |app: &App| app.shown[app.list.selected().unwrap()].flags.flagged;
    let was = flagged(&app);
    press(&mut app, &["!"]);
    assert_eq!(flagged(&app), !was);
    press(&mut app, &["!"]);
    assert_eq!(flagged(&app), was);

    // S: straight to the folder; esc, back to the message, no list shown.
    press(&mut app, &["S"]);
    assert!(app.dialog.saving().is_some(), "no folder asked");
    press(&mut app, &["<esc>"]);
    assert!(!app.dialog.is_open(), "a list never opened");

    // All of them saved there.
    press(&mut app, &["S"]);
    assert!(app.dialog.saving().is_some(), "no folder asked");
    press(&mut app, &["<enter>"]);
    assert!(dir.path().join("states.txt").is_file());
    assert!(dir.path().join("computable-numbers.pdf").is_file());
    assert!(!app.dialog.is_open(), "{}", app.status);
}

#[test]
fn the_folder_to_save_in_shows_its_end_with_one_slash() {
    let mut app = demo();
    let dir = tempfile::tempdir().unwrap();
    let deep = dir
        .path()
        .join("a-folder-of-a-long-name-to-push-the-end-out-of-sight")
        .join("and-another-as-long-as-the-first")
        .join("alpha");
    std::fs::create_dir_all(&deep).unwrap();
    app.save_dir = Some(deep);
    read_turing(&mut app);
    press(&mut app, &["S"]);
    let screen = app::screen(&mut app, 80, 24);
    let header = screen
        .lines()
        .find(|l| l.starts_with("│ …"))
        .expect("its start cut");
    let end = format!("{}alpha/ ", std::path::MAIN_SEPARATOR);
    assert!(header.contains(&end), "{header}");

    // At the root, a slash alone.
    if cfg!(windows) {
        return;
    }
    press(&mut app, &["<esc>"]);
    app.save_dir = Some(PathBuf::from("/"));
    press(&mut app, &["S"]);
    let screen = app::screen(&mut app, 80, 24);
    assert!(screen.contains("│ /  "), "{screen}");
    assert!(!screen.contains("//"), "{screen}");
}
