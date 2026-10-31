//! Where the message read is put: beside the list, below it, out of sight
//! until read, or over the list.

use super::*;
use crate::app::Placement;

/// Whether the sidebar is drawn: its Inbox in its first columns.
fn sidebar(screen: &str) -> bool {
    screen
        .lines()
        .any(|l| l.chars().take(20).collect::<String>().contains("Inbox"))
}

/// The row of the screen a line holding `text` is on, and its column.
fn found(screen: &str, text: &str) -> Option<(usize, usize)> {
    screen
        .lines()
        .enumerate()
        .find_map(|(y, l)| l.find(text).map(|x| (y, l[..x].chars().count())))
}

#[test]
fn backslash_turns_where_the_message_read_is_and_says_so() {
    let mut app = demo();
    assert_eq!(app.placement, Placement::Right);
    for (want, said) in [
        (Placement::Below, "below"),
        (Placement::Hidden, "hidden"),
        (Placement::Popup, "pop-up"),
        (Placement::Right, "right"),
    ] {
        press(&mut app, &["\\"]);
        assert_eq!(app.placement, want);
        assert!(app.status.contains(said), "{}", app.status);
    }
}

#[test]
fn below_the_message_read_is_under_the_list_and_follows_it() {
    let mut app = demo();
    press(&mut app, &["\\", "<enter>"]);
    let screen = app::screen(&mut app, 140, 40);
    let (from_y, from_x) = found(&screen, "From  Ada Lovelace").expect("read");
    let (row_y, _) = found(&screen, "Ada Lovelace").expect("listed");
    assert!(row_y < from_y, "{screen}");
    // As wide as the list, the sidebar still beside them.
    assert!(from_x < 40, "{screen}");
    assert!(app.read_width > 100, "{}", app.read_width);
    assert!(sidebar(&screen), "{screen}");
    // The next one picked is read, as beside.
    press(&mut app, &["<s-tab>", "j"]);
    app.settle(Duration::from_secs(5));
    assert_eq!(
        app.message().unwrap().subject,
        "[letterbox] Issue #41: a map of the inbox"
    );
    // Also in a window too narrow to have it beside.
    let screen = app::screen(&mut app, 100, 40);
    assert!(app.read_width > 70, "{screen}");
}

#[test]
fn below_its_line_is_dragged_and_the_keys_make_it_taller() {
    let mut app = demo();
    press(&mut app, &["\\", "<enter>"]);
    let _ = app::screen(&mut app, 140, 40);
    let tall = app.read_height;
    press(&mut app, &["<"]);
    let _ = app::screen(&mut app, 140, 40);
    assert_eq!(app.read_height, tall + 2);
    press(&mut app, &[">", ">"]);
    let _ = app::screen(&mut app, 140, 40);
    assert_eq!(app.read_height, tall - 2);
    let y = app.panes.bottom() - app.read_height;
    app.mouse(mouse(MouseEventKind::Down(MouseButton::Left), 60, y));
    assert!(app.resizing);
    app.mouse(mouse(MouseEventKind::Drag(MouseButton::Left), 60, y - 5));
    app.mouse(mouse(MouseEventKind::Up(MouseButton::Left), 60, y - 5));
    let _ = app::screen(&mut app, 140, 40);
    assert_eq!(app.read_height, tall + 3);
    assert!(!app.resizing);
}

#[test]
fn hidden_the_list_has_the_room_and_a_message_read_has_the_screen() {
    let mut app = demo();
    press(&mut app, &["\\", "\\"]);
    let _ = app::screen(&mut app, 140, 40);
    assert_eq!(app.read_width, 0);
    // Moving along the list reads nothing.
    press(&mut app, &["j"]);
    assert!(app.reading.is_none());
    press(&mut app, &["<enter>"]);
    assert_eq!(app.focus, Focus::Reader);
    let screen = app::screen(&mut app, 140, 40);
    assert_eq!(app.read_width, 140, "{screen}");
    press(&mut app, &["<esc>"]);
    assert_eq!(app.focus, Focus::List);
    let _ = app::screen(&mut app, 140, 40);
    assert_eq!(app.read_width, 0);
}

#[test]
fn as_a_pop_up_the_message_read_is_over_the_list_until_esc() {
    let mut app = demo();
    press(&mut app, &["\\", "\\", "\\"]);
    let _ = app::screen(&mut app, 140, 40);
    assert_eq!(app.read_width, 0);
    press(&mut app, &["<enter>"]);
    let screen = app::screen(&mut app, 140, 40);
    assert!(screen.contains("From  Ada Lovelace"), "{screen}");
    // Over the list, the sidebar seen beside it.
    assert!(app.read_width > 60 && app.read_width < 130, "{screen}");
    assert!(sidebar(&screen), "{screen}");
    assert!(screen.contains('╭'), "{screen}");
    press(&mut app, &["<esc>"]);
    let screen = app::screen(&mut app, 140, 40);
    assert_eq!(app.read_width, 0);
    assert!(!screen.contains("From  Ada Lovelace"), "{screen}");
}

#[test]
fn where_the_message_read_was_put_is_where_it_opens() {
    let dir = tempfile::tempdir().unwrap();
    let look = dir.path().join("look");
    let mut app = demo();
    app.look_file = Some(look.clone());
    press(&mut app, &["\\"]);
    let mut again = demo();
    again.look_file = Some(look);
    again.recall_look();
    assert_eq!(again.placement, Placement::Below);
}

#[test]
fn below_on_a_short_screen_it_is_still_drawn() {
    let mut app = demo();
    press(&mut app, &["\\", "<enter>"]);
    for rows in 4..16 {
        let _ = app::screen(&mut app, 120, rows);
    }
}

#[test]
fn hidden_n_reads_the_next_and_shows_it() {
    let mut app = demo();
    press(&mut app, &["\\", "\\", "n"]);
    assert!(app.reading.is_some());
    assert_eq!(app.focus, Focus::Reader);
    // Put out of sight, nothing is read as the list is moved along, even
    // before the screen is drawn again.
    let mut app = demo();
    press(&mut app, &["\\"]);
    let _ = app::screen(&mut app, 140, 40);
    for key in ["\\", "j"] {
        app.key(app::keys(key)[0]);
    }
    app.settle(Duration::from_secs(2));
    assert!(app.reading.is_none());
}

#[test]
fn a_click_on_the_pop_ups_edge_is_its_not_the_lists() {
    let mut app = demo();
    press(&mut app, &["\\", "\\", "\\", "<enter>"]);
    let _ = app::screen(&mut app, 140, 40);
    let picked = app.list.selected();
    // Its foot, over a row of the list far from the one read.
    let screen = app::screen(&mut app, 140, 40);
    let (y, _) = found(&screen, "╰").expect("its foot");
    let y = y as u16;
    app.mouse(mouse(MouseEventKind::Down(MouseButton::Left), 70, y));
    app.mouse(mouse(MouseEventKind::Up(MouseButton::Left), 70, y));
    assert_eq!(app.list.selected(), picked);
    assert_eq!(app.focus, Focus::Reader);
}

#[test]
fn the_inbox_looked_through_stays_beside_the_map_wherever_the_message_is_put() {
    for n in 2..=3 {
        let mut app = demo_map();
        for _ in 0..n {
            press(&mut app, &["\\"]);
        }
        press(&mut app, &["i"]);
        let _ = app::screen(&mut app, 140, 40);
        assert!(app.review.is_some());
        assert!(
            app.read_width > 0 && app.read_width < 120,
            "{}",
            app.read_width
        );
    }
}

#[test]
fn too_narrow_to_put_it_elsewhere_backslash_says_so() {
    let mut app = demo();
    let _ = app::screen(&mut app, 60, 30);
    app.key(app::keys("\\")[0]);
    assert!(app.status.contains("narrow"), "{}", app.status);
}
