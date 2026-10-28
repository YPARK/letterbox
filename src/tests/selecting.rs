//! Several messages are selected with the mouse, never by one key pressed:
//! dragged over in the list or the Inbox looked through, a row added or
//! taken by Ctrl and a click, or as far as one clicked with Shift.
use super::mapscreen::demo_map;
use super::*;

/// Where `target` is on the screen as it is drawn now.
fn spot(app: &mut App, target: app::Target) -> (u16, u16) {
    let _ = app::screen(app, 140, 40);
    let (rect, _) = *app
        .targets
        .iter()
        .rev()
        .find(|(_, t)| *t == target)
        .unwrap_or_else(|| panic!("no {target:?} on the screen"));
    (rect.x + 1, rect.y)
}

fn event(kind: MouseEventKind, (column, row): (u16, u16), modifiers: KeyModifiers) -> MouseEvent {
    MouseEvent {
        kind,
        column,
        row,
        modifiers,
    }
}

/// The left button pressed on `from`, dragged to `to`, and let go.
pub(super) fn drag(app: &mut App, from: app::Target, to: app::Target) {
    let a = spot(app, from);
    let b = spot(app, to);
    app.mouse(event(
        MouseEventKind::Down(MouseButton::Left),
        a,
        KeyModifiers::NONE,
    ));
    app.mouse(event(
        MouseEventKind::Drag(MouseButton::Left),
        b,
        KeyModifiers::NONE,
    ));
    app.mouse(event(
        MouseEventKind::Up(MouseButton::Left),
        b,
        KeyModifiers::NONE,
    ));
    app.settle(Duration::from_secs(5));
}

/// The list's rows from `a` to `b` selected, dragged over.
pub(super) fn select_rows(app: &mut App, a: usize, b: usize) {
    drag(app, app::Target::Message(a), app::Target::Message(b));
}

/// The list's row `i` added to what is selected, or taken away: Ctrl and
/// a click.
pub(super) fn ctrl_click(app: &mut App, i: usize) {
    click_with(app, app::Target::Message(i), KeyModifiers::CONTROL);
}

/// A click on `at` with `held` held.
fn click_with(app: &mut App, at: app::Target, held: KeyModifiers) {
    let p = spot(app, at);
    app.mouse(event(MouseEventKind::Down(MouseButton::Left), p, held));
    app.mouse(event(MouseEventKind::Up(MouseButton::Left), p, held));
    app.settle(Duration::from_secs(5));
}

fn selected(app: &App) -> usize {
    app.map.region.as_ref().map_or(0, |r| r.keys.len())
}

#[test]
fn in_the_list_v_and_space_select_nothing() {
    let mut app = demo();
    press(&mut app, &["v"]);
    assert_eq!(selected(&app), 0);
    assert!(app.status.contains("mouse"), "said how: {}", app.status);
    press(&mut app, &[" "]);
    assert_eq!(selected(&app), 0);
}

#[test]
fn a_plain_click_selects_nothing() {
    let mut app = demo();
    click_with(&mut app, app::Target::Message(2), KeyModifiers::NONE);
    assert_eq!(selected(&app), 0);
    assert_eq!(app.list.selected(), Some(2));
}

#[test]
fn dragging_over_rows_in_the_list_selects_them() {
    let mut app = demo();
    drag(&mut app, app::Target::Message(0), app::Target::Message(2));
    assert_eq!(selected(&app), 3, "{}", app.status);
    let screen = app::screen(&mut app, 140, 40);
    assert!(screen.contains("▪ 3 marked"), "{screen}");
    // And d asks first, as for any several.
    press(&mut app, &["d"]);
    assert!(matches!(app.dialog, app::Dialog::Bulk(_)));
}

#[test]
fn ctrl_and_a_click_add_a_row_or_take_it_away() {
    let mut app = demo();
    click_with(&mut app, app::Target::Message(1), KeyModifiers::CONTROL);
    assert_eq!(selected(&app), 1);
    click_with(&mut app, app::Target::Message(3), KeyModifiers::CONTROL);
    assert_eq!(selected(&app), 2);
    click_with(&mut app, app::Target::Message(1), KeyModifiers::CONTROL);
    assert_eq!(selected(&app), 1);
}

#[test]
fn shift_and_a_click_select_as_far_as_it_from_the_row_picked() {
    let mut app = demo();
    click_with(&mut app, app::Target::Message(1), KeyModifiers::NONE);
    click_with(&mut app, app::Target::Message(4), KeyModifiers::SHIFT);
    assert_eq!(selected(&app), 4);
}

#[test]
fn in_the_inbox_looked_through_dragging_selects_rows() {
    let mut app = demo_map();
    press(&mut app, &["i"]);
    assert!(app.review.is_some());
    drag(
        &mut app,
        app::Target::ReviewRow(0),
        app::Target::ReviewRow(2),
    );
    let n = app.review.as_ref().map_or(0, |r| r.selected.len());
    assert_eq!(n, 3, "{}", app.status);
}

#[test]
fn the_lists_keys_below_offer_no_key_that_selects() {
    let mut app = demo();
    let screen = app::screen(&mut app, 200, 40);
    let foot: String = screen.lines().rev().take(3).collect::<Vec<_>>().join("\n");
    assert!(!foot.contains("select"), "{foot}");
}
