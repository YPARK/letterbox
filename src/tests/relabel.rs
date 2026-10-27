//! A label renamed from the map: by R, or a right-click on its anchor;
//! asked first, how many carry it; never onto a label there is; and put
//! back by C-x u.
use super::mapscreen::demo_map;
use super::*;

/// The number of the cluster labelled `label`.
fn cluster_of(app: &App, label: &str) -> u32 {
    let s = app.map.snapshot.as_ref().unwrap();
    s.groups
        .iter()
        .find(|g| g.label.as_deref() == Some(label))
        .unwrap()
        .id
}

fn right_click(app: &mut App, target: app::Target) {
    let _ = app::screen(app, 150, 45);
    let (rect, _) = *app
        .targets
        .iter()
        .rev()
        .find(|(_, t)| *t == target)
        .unwrap_or_else(|| panic!("no {target:?} on the screen"));
    app.mouse(MouseEvent {
        kind: MouseEventKind::Down(MouseButton::Right),
        column: rect.x,
        row: rect.y,
        modifiers: KeyModifiers::NONE,
    });
    app.settle(Duration::from_secs(5));
}

#[test]
fn a_right_click_on_a_labels_anchor_offers_to_rename_it() {
    let mut app = demo_map();
    let grant = cluster_of(&app, "Grant");
    right_click(&mut app, app::Target::Anchor(grant));
    assert!(
        matches!(app.dialog, app::Dialog::ClusterMenu { .. }),
        "{}",
        app.status
    );
    let screen = app::screen(&mut app, 150, 45);
    assert!(screen.contains("Rename"), "{screen}");
    press(&mut app, &["<enter>"]);
    let app::Dialog::RenameLabel(old, typed) = &app.dialog else {
        panic!("not asked the name: {}", app.status);
    };
    assert_eq!((old.as_str(), typed.as_str()), ("Grant", "Grant"));
}

#[test]
fn a_rename_is_asked_first_and_put_back_by_c_x_u() {
    let mut app = demo_map();
    super::filing::pick_subject(&mut app, "Letter of support, signed");
    press(&mut app, &["R", "<c-u>", "Funding", "<enter>"]);
    assert!(
        matches!(app.dialog, app::Dialog::ConfirmRename { .. }),
        "asked first: {}",
        app.status
    );
    let screen = app::screen(&mut app, 150, 45);
    assert!(
        screen.contains("Grant") && screen.contains("Funding"),
        "{screen}"
    );
    // Anything but y keeps it.
    press(&mut app, &["n"]);
    let (labels, _) = dot_labels(&app, "Letter of support, signed");
    assert!(labels.contains(&"Grant".to_string()), "{labels:?}");
    // y renames it.
    press(&mut app, &["R", "<c-u>", "Funding", "<enter>", "y"]);
    let (labels, _) = dot_labels(&app, "Letter of support, signed");
    assert!(labels.contains(&"Funding".to_string()), "{labels:?}");
    assert!(app.status.contains("C-x u"), "said how: {}", app.status);
    // C-x u: back as it was.
    press(&mut app, &["<c-x>", "u"]);
    let (labels, _) = dot_labels(&app, "Letter of support, signed");
    assert!(labels.contains(&"Grant".to_string()), "{labels:?}");
    assert!(!labels.contains(&"Funding".to_string()), "{labels:?}");
}

#[test]
fn a_label_is_never_renamed_onto_one_there_is() {
    let mut app = demo_map();
    super::filing::pick_subject(&mut app, "Letter of support, signed");
    press(&mut app, &["R", "<c-u>", "Projects", "<enter>"]);
    assert!(!app.dialog.is_open(), "{}", app.status);
    assert!(app.status.contains("already"), "{}", app.status);
    let (labels, _) = dot_labels(&app, "Letter of support, signed");
    assert!(labels.contains(&"Grant".to_string()), "{labels:?}");
}
