//! Filing, from the map.

use super::*;

// ---- filing, from the map ----

/// Down the cluster's tree to the message of the map with `subject`: the
/// map acts on a message only there.
pub(super) fn down_subject(app: &mut App, subject: &str) {
    pick_subject(app, subject);
    let key = app.map.cursor.clone().unwrap();
    super::mapsafety::down_to(app, &key);
}

/// Pick the message of the map with `subject`.
pub(super) fn pick_subject(app: &mut App, subject: &str) {
    let s = app.map.snapshot.clone().unwrap();
    let i = s
        .dots
        .iter()
        .position(|d| d.subject == subject)
        .unwrap_or_else(|| panic!("no {subject}"));
    app.map.cursor = Some(s.dots[i].key.clone());
}

#[test]
fn keys_mean_the_same_on_the_map_as_in_the_list() {
    let mut app = demo_map();
    // r answers the message picked on the map, not the list's.
    down_subject(&mut app, "Aims page, third pass");
    press(&mut app, &["r"]);
    let text = app.compose.as_ref().expect("written").buffer.text();
    assert!(
        text.contains("Subject: Re: Aims page, third pass"),
        "{text}"
    );
    press(&mut app, &["<c-c>", "<c-k>"]);
    // e archives it, as it archives in the list.
    press(&mut app, &["e"]);
    let (_, inbox) = dot_labels(&app, "Aims page, third pass");
    assert!(!inbox, "{}", app.status);
    // M, back to the list.
    press(&mut app, &["M"]);
    assert_eq!(app.map.mode, crate::map::Mode::List);
    let first = app.shown[0].summary.subject.clone();
    press(&mut app, &["e"]);
    assert!(
        app.status.starts_with("archived, out of the Inbox"),
        "{}",
        app.status
    );
    // Out of the Inbox; listed still where the list is of its cluster.
    assert!(!dot_labels(&app, &first).1, "{first} out of the Inbox");
}

#[test]
fn the_newest_file_of_several_as_new_is_the_first_by_name() {
    let dir = tempfile::tempdir().unwrap();
    let at = std::time::SystemTime::now() - Duration::from_secs(60);
    for name in ["b.txt", "a.txt", "c.txt"] {
        let path = dir.path().join(name);
        std::fs::write(&path, b"x").unwrap();
        std::fs::File::options()
            .write(true)
            .open(&path)
            .unwrap()
            .set_modified(at)
            .unwrap();
    }
    let p = crate::pick::Picker::open(dir.path().to_path_buf());
    assert_eq!(
        p.current().map(|e| e.name().to_string()).as_deref(),
        Some("a.txt")
    );
}

#[test]
fn space_marks_in_the_picker_after_typing_too() {
    let dir = tempfile::tempdir().unwrap();
    // Each newer than the last, as a clock that tells them apart has it.
    let at = std::time::SystemTime::now() - Duration::from_secs(60);
    for (i, name) in ["one draft.txt", "two draft.txt", "notes.txt"]
        .iter()
        .enumerate()
    {
        let path = dir.path().join(name);
        std::fs::write(&path, b"x").unwrap();
        std::fs::File::options()
            .write(true)
            .open(&path)
            .unwrap()
            .set_modified(at + Duration::from_secs(i as u64))
            .unwrap();
    }
    let mut p = crate::pick::Picker::open(dir.path().to_path_buf());
    for c in "draft".chars() {
        p.key(KeyEvent::new(KeyCode::Char(c), KeyModifiers::NONE));
    }
    assert_eq!(p.shown().len(), 3, "..: and the two drafts");
    p.key(KeyEvent::new(KeyCode::Char(' '), KeyModifiers::NONE));
    p.key(KeyEvent::new(KeyCode::Char(' '), KeyModifiers::NONE));
    assert_eq!(p.filter, "draft");
    let crate::pick::Picked::Take(files) = p.key(KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE))
    else {
        panic!("none taken");
    };
    let names: Vec<_> = files
        .iter()
        .map(|f| f.file_name().unwrap().to_owned())
        .collect();
    assert_eq!(names, ["one draft.txt", "two draft.txt"]);
}

#[test]
fn a_search_on_the_map_lights_what_holds_it_and_n_goes_through_them() {
    let mut app = demo_map();
    press(&mut app, &["/", "biosketch"]);
    let s = app.map.snapshot.clone().unwrap();
    let found: Vec<usize> = (0..s.dots.len())
        .filter(|&i| app.map.hits.as_ref().unwrap().matched[i])
        .collect();
    assert_eq!(found.len(), 2, "Mira's, and your answer");
    assert!(
        found
            .iter()
            .all(|&i| s.dots[i].subject.to_lowercase().contains("biosketch"))
    );
    // The one picked is one found, as it is typed.
    assert!(found.contains(&app.map.picked().unwrap()));
    let screen = app::screen(&mut app, 150, 45);
    assert!(screen.contains("2 found for “biosketch”"), "{screen}");
    // ⏎ asks no server; n goes to the other, and round again.
    press(&mut app, &["<enter>"]);
    assert!(!app.searching);
    assert_eq!(app.map.mode, crate::map::Mode::Map);
    let first = app.map.picked().unwrap();
    press(&mut app, &["n"]);
    let second = app.map.picked().unwrap();
    assert_ne!(first, second);
    assert!(found.contains(&second));
    press(&mut app, &["n"]);
    assert_eq!(app.map.picked().unwrap(), first);
    // Of two words, both; and esc lets it go.
    press(&mut app, &["/", "<c-u>", "mira budget"]);
    let now = jiff::Timestamp::now().as_second();
    assert_eq!(app.map.found("mira budget", now).len(), 1);
    press(&mut app, &["<enter>", "<esc>"]);
    assert!(app.query.is_empty());
    assert!(app.map.hits.is_none());
}

#[test]
fn another_folder_listed_leaves_the_message_read_behind() {
    let mut app = demo();
    press(&mut app, &["<enter>"]);
    let subject = app.message().expect("read").subject.clone();
    let screen = app::screen(&mut app, 150, 40);
    assert!(screen.contains(&subject), "{screen}");
    // Sent, from the sidebar: the message of the Inbox is not beside it.
    while app.focus != app::Focus::Side {
        press(&mut app, &["h"]);
    }
    press(&mut app, &["j", "j"]);
    assert_eq!(app.folder_name(), "Sent");
    assert!(app.reading.is_none());
    let screen = app::screen(&mut app, 150, 40);
    assert!(!screen.contains(&subject), "{screen}");
}

#[test]
fn the_line_before_the_message_is_dragged_to_make_it_wider() {
    let mut app = demo();
    let divider = |app: &mut App| {
        let _ = app::screen(app, 150, 40);
        app.targets
            .iter()
            .find(|(_, t)| *t == app::Target::Divider)
            .map(|(r, _)| r.x + 1)
            .expect("a line to drag")
    };
    let x = divider(&mut app);
    app.mouse(mouse(MouseEventKind::Down(MouseButton::Left), x, 10));
    app.mouse(mouse(MouseEventKind::Drag(MouseButton::Left), x - 20, 10));
    app.mouse(mouse(MouseEventKind::Up(MouseButton::Left), x - 20, 10));
    assert!(!app.resizing);
    assert_eq!(divider(&mut app), x - 20, "the message twenty wider");
    // The map keeps its width too.
    press(&mut app, &["M"]);
    assert_eq!(divider(&mut app), x - 20);
    press(&mut app, &["M"]);
    // Not past what leaves the list room.
    app.mouse(mouse(MouseEventKind::Down(MouseButton::Left), x - 20, 10));
    app.mouse(mouse(MouseEventKind::Drag(MouseButton::Left), 0, 10));
    app.mouse(mouse(MouseEventKind::Up(MouseButton::Left), 0, 10));
    let narrowest = divider(&mut app);
    assert!(
        narrowest >= 24 + 24,
        "past the sidebar, the list 24 wide: {narrowest}"
    );
    // A column beside the line takes it too; < and > move it by four.
    app.mouse(mouse(
        MouseEventKind::Down(MouseButton::Left),
        narrowest + 1,
        10,
    ));
    assert!(app.resizing);
    app.mouse(mouse(
        MouseEventKind::Up(MouseButton::Left),
        narrowest + 1,
        10,
    ));
    // Each as wide as the screen it was pressed on: this one, 150 wide.
    let char_key = |c| KeyEvent::new(KeyCode::Char(c), KeyModifiers::NONE);
    app.key(char_key('>'));
    assert_eq!(divider(&mut app), narrowest + 4);
    app.key(char_key('<'));
    assert_eq!(divider(&mut app), narrowest);
}

/// The labels of the map's message with `subject`, and whether it is in
/// the Inbox.
pub(super) fn dot_labels(app: &App, subject: &str) -> (Vec<String>, bool) {
    let s = app.map.snapshot.as_ref().unwrap();
    let d = s.dots.iter().find(|d| d.subject == subject).unwrap();
    (d.labels.clone(), d.system.inbox)
}

#[test]
fn m_moves_a_message_onto_a_cluster_and_files_it_under_its_label() {
    let mut app = demo_map();
    down_subject(&mut app, "Aims page, third pass");
    press(&mut app, &["m"]);
    assert!(app.map.moving.is_some());
    assert!(app.status.contains("moving it"), "{}", app.status);
    // Onto the Grant's cluster, by a message of it, and ⏎.
    pick_subject(&mut app, "Letter of support, signed");
    press(&mut app, &["<enter>"]);
    assert!(app.map.moving.is_none());
    assert!(app.status.contains("under Grant"), "{}", app.status);
    let (labels, inbox) = dot_labels(&app, "Aims page, third pass");
    assert!(labels.contains(&"Grant".to_string()), "{labels:?}");
    assert!(!inbox, "out of the Inbox");
    // The server has it, the next pass says, and it is pending no more.
    assert!(app.index.lock().unwrap().keep.pending.is_empty());
    let s = app.map.snapshot.clone().unwrap();
    let aims = s
        .dots
        .iter()
        .find(|d| d.subject == "Aims page, third pass")
        .unwrap();
    assert!(aims.places.iter().any(|p| p.folder == "Archive"));
    // Its like are suggested, beside it, read: its cluster gone into, down
    // to it.
    read_on_map(&mut app);
    let screen = app::screen(&mut app, 150, 60);
    assert!(screen.contains("Would go to Grant too"), "{screen}");
}

#[test]
fn n_files_what_is_moving_under_a_new_label_named_by_its_commonest_word() {
    let mut app = demo_map();
    // Several, as they are filed: selected in the list, asked first.
    press(&mut app, &["M"]);
    let s = app.map.snapshot.clone().unwrap();
    let builds: Vec<crate::index::Key> = s
        .dots
        .iter()
        .filter(|d| d.address == "ci@runner.example")
        .map(|d| d.key.clone())
        .collect();
    app.map.region = Some(crate::map::Region { keys: builds });
    press(&mut app, &["N"]);
    let app::Dialog::Label(typed) = &app.dialog else {
        panic!("not asked: {}", app.status);
    };
    assert_eq!(typed, "Build");
    press(&mut app, &["s", "<enter>"]);
    assert!(
        matches!(app.dialog, app::Dialog::Bulk(_)),
        "asked: {}",
        app.status
    );
    press(&mut app, &["y"]);
    assert!(
        app.status.contains("3 filed under Builds"),
        "{}",
        app.status
    );
    for subject in [
        "Build failed: letterbox main (clippy)",
        "Build passed: letterbox main",
        "Build failed: faba (test timeout)",
    ] {
        assert_eq!(dot_labels(&app, subject).0, ["Builds"]);
    }
    // A cluster that is a label now, its anchor said so.
    let s = app.map.snapshot.clone().unwrap();
    assert!(
        s.groups
            .iter()
            .any(|g| g.label.as_deref() == Some("Builds"))
    );
}

#[test]
fn filing_suggests_its_like_and_y_and_minus_take_them_or_not() {
    let mut app = demo_map();
    down_subject(&mut app, "R01 resubmission: internal deadline 3 November");
    press(&mut app, &["m"]);
    // Onto the Grant's anchor, by a click: out to all the clusters, where
    // it goes picked there.
    press(&mut app, &["0"]);
    assert!(app.map.moving.is_some(), "still moving");
    let _ = app::screen(&mut app, 150, 45);
    let s = app.map.snapshot.clone().unwrap();
    let grant = s
        .groups
        .iter()
        .find(|g| g.label.as_deref() == Some("Grant"))
        .unwrap()
        .id;
    click_at(&mut app, app::Target::Anchor(grant));
    assert!(app.status.contains("under Grant"), "{}", app.status);
    let suggested: Vec<String> = {
        let s = app.map.snapshot.clone().unwrap();
        app.suggestions()
            .iter()
            .map(|&(i, _)| s.dots[i].subject.clone())
            .collect()
    };
    assert!(
        suggested.contains(&"Budget justification: the sequencing line".to_string()),
        "{suggested:?}"
    );
    let first = suggested[0].clone();
    // Taken down at a message, where the map acts: the one filed.
    down_subject(&mut app, "R01 resubmission: internal deadline 3 November");
    press(&mut app, &["y"]);
    assert!(dot_labels(&app, &first).0.contains(&"Grant".to_string()));
    let next = {
        let s = app.map.snapshot.clone().unwrap();
        s.dots[app.suggestions()[0].0].subject.clone()
    };
    press(&mut app, &["-"]);
    assert!(app.status.contains("suggested no more"), "{}", app.status);
    let s = app.map.snapshot.clone().unwrap();
    assert!(
        app.suggestions()
            .iter()
            .all(|&(i, _)| s.dots[i].subject != next)
    );
    assert_eq!(app.index.lock().unwrap().keep.not_it.len(), 1);
    press(&mut app, &["<esc>"]);
    assert!(app.map.suggesting.is_none());
}

#[test]
fn r_down_at_a_message_offers_to_rename_split_or_remove_its_label() {
    let mut app = demo_map();
    down_subject(&mut app, "Letter of support, signed");
    press(&mut app, &["R"]);
    assert!(
        matches!(app.dialog, app::Dialog::ClusterMenu { at: 0, .. }),
        "{}",
        app.status
    );
    let screen = app::screen(&mut app, 150, 45);
    assert!(screen.contains("Split into subfolders"), "{screen}");
    press(&mut app, &["<enter>"]);
    assert!(
        matches!(&app.dialog, app::Dialog::RenameLabel(old, _) if old == "Grant"),
        "{}",
        app.status
    );
}

#[test]
fn r_renames_the_label_picked_on_its_server() {
    let mut app = demo_map();
    pick_subject(&mut app, "Letter of support, signed");
    press(&mut app, &["R"]);
    let app::Dialog::RenameLabel(old, typed) = &app.dialog else {
        panic!("not asked: {}", app.status);
    };
    assert_eq!((old.as_str(), typed.as_str()), ("Grant", "Grant"));
    press(&mut app, &["<c-u>", "Funding", "<enter>", "y"]);
    assert!(
        app.status.contains("renamed Grant to Funding"),
        "{}",
        app.status
    );
    let (labels, _) = dot_labels(&app, "Letter of support, signed");
    assert_eq!(labels, ["Projects", "Funding"]);
    // Outside the map, R renames an account, as it did.
    press(&mut app, &["M", "R"]);
    assert!(matches!(app.dialog, app::Dialog::Rename(0, _)));
}
