//! Quitting while mail is still being changed asks first: a move, a label
//! taken off, a folder made or a message sent, cut off halfway, is worse
//! than a second key.
use super::*;

/// Keys pressed as they come, not waiting for the servers to answer.
fn keys_now(app: &mut App, keys: &[&str]) {
    for key in keys.iter().flat_map(|k| app::keys(k)) {
        app.key(key);
    }
}

#[test]
fn quitting_while_mail_is_moved_asks_first() {
    let mut app = demo();
    // d: a move asked of the server, not yet answered.
    keys_now(&mut app, &["d", "<c-x>", "<c-c>"]);
    assert!(!app.quit, "asked first: {}", app.status);
    assert!(app.status.contains("still"), "{}", app.status);
    assert!(app.status.contains("y quits"), "{}", app.status);
    // Anything but y waits.
    keys_now(&mut app, &["n"]);
    assert!(!app.quit);
    // y quits anyway.
    keys_now(&mut app, &["<c-x>", "<c-c>", "y"]);
    assert!(app.quit);
}

#[test]
fn quitting_with_nothing_being_changed_is_at_once() {
    let mut app = demo();
    press(&mut app, &["d"]);
    app.settle(Duration::from_secs(5));
    keys_now(&mut app, &["<c-x>", "<c-c>"]);
    assert!(app.quit, "{}", app.status);
}
