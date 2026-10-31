//! The foot and the status line say what each key does where it is
//! pressed, and what was done, no longer than it is so.
use super::*;

/// The demo, `keys` pressed on a screen `width` by `height`, drawn before
/// each as a terminal is; and the screen after.
fn at(width: u16, height: u16, keys: &[&str]) -> (App, String) {
    let mut app = demo();
    for key in keys.iter().flat_map(|k| app::keys(k)) {
        let _ = app::screen(&mut app, width, height);
        app.key(key);
        app.settle(Duration::from_secs(5));
    }
    let screen = app::screen(&mut app, width, height);
    (app, screen)
}

/// The foot's last two lines.
fn foot(screen: &str) -> String {
    let lines: Vec<&str> = screen.lines().collect();
    lines[lines.len().saturating_sub(2)..].join("\n")
}

#[test]
fn size_is_offered_only_where_it_sizes_and_said_where_it_does_not() {
    // Beside the list, < and > size it.
    let (_, screen) = at(120, 35, &["<enter>"]);
    assert!(foot(&screen).contains("</> size"), "{screen}");
    // Over the list, nothing to size: not offered, and said so if pressed.
    let (app, screen) = at(120, 35, &["\\", "\\", "\\", "<enter>", "<"]);
    assert!(!foot(&screen).contains("</>"), "{screen}");
    assert!(app.status.contains("beside the list"), "{}", app.status);
}

#[test]
fn esc_back_is_kept_when_the_message_fills_a_narrow_window() {
    for (w, h) in [(80, 24), (70, 30)] {
        let (_, screen) = at(w, h, &["<enter>"]);
        assert!(foot(&screen).contains("esc back"), "{w}x{h}:\n{screen}");
        assert!(foot(&screen).contains("C-x C-c quit"), "{screen}");
    }
}

#[test]
fn in_trash_d_is_said_to_put_back() {
    let (_, screen) = at(120, 30, &["d", "h", "j", "j", "j", "j", "j", "l"]);
    assert!(foot(&screen).contains("d put back"), "{screen}");
    assert!(!foot(&screen).contains("d trash"), "{screen}");
    let (_, screen) = at(120, 30, &["d", "h", "j", "j", "j", "j", "j", "<enter>"]);
    assert!(foot(&screen).contains("d put back"), "{screen}");
    // In the Inbox, d trashes.
    let (_, screen) = at(120, 30, &[]);
    assert!(foot(&screen).contains("d trash"), "{screen}");
}

#[test]
fn a_move_out_of_drafts_is_confirmed() {
    let (app, _) = at(120, 30, &["h", "j", "l", "e", "j", "k"]);
    assert!(!app.batch.busy(), "{}", app.batch.said());
    assert!(app.status.contains("✓ confirmed"), "{}", app.status);
    assert!(app.status.contains("out of Drafts"), "{}", app.status);
}

#[test]
fn archived_from_sent_says_out_of_sent() {
    let (app, _) = at(120, 30, &["h", "j", "j", "l", "e"]);
    assert!(
        app.status.contains("archived, out of Sent"),
        "{}",
        app.status
    );
}

#[test]
fn while_the_search_is_typed_the_foot_says_its_keys() {
    let (_, screen) = at(120, 30, &["/", "gra"]);
    let f = foot(&screen);
    assert!(f.contains("⏎ every folder"), "{screen}");
    assert!(f.contains("esc clear"), "{screen}");
    assert!(
        !f.contains("d trash") && !f.contains("c compose"),
        "{screen}"
    );
    // Down to the list, its keys again.
    let (_, screen) = at(120, 30, &["/", "gra", "<down>"]);
    assert!(foot(&screen).contains("d trash"), "{screen}");
}

#[test]
fn looking_for_new_mail_is_said_done() {
    let (app, _) = at(120, 30, &["U"]);
    assert_eq!(app.status, "no new mail");
}

#[test]
fn new_mail_is_not_claimed_of_an_inbox_not_counted_before() {
    let mut app = demo();
    let inbox = app.accounts[0]
        .folders
        .iter()
        .position(|f| f.role == mail::Role::Inbox)
        .expect("an Inbox");
    app.accounts[0].folders[inbox].total = None;
    press(&mut app, &["U"]);
    app.settle(Duration::from_secs(5));
    assert!(app.accounts[0].folders[inbox].total.is_some(), "counted");
    assert_eq!(app.status, "no new mail");
}

#[test]
fn looking_for_new_mail_with_no_account_says_so_at_once() {
    let mut app = App::new(Vec::new(), PathBuf::from("/nowhere/accounts.toml"));
    press(&mut app, &["U"]);
    assert_eq!(app.status, "no account to look in");
}

#[test]
fn hidden_until_enter_is_not_said_once_one_is_read() {
    let (app, _) = at(120, 30, &["\\", "\\"]);
    assert!(app.status.contains("hidden until"), "{}", app.status);
    let (app, _) = at(120, 30, &["\\", "\\", "<enter>"]);
    assert!(!app.status.contains("hidden until"), "{}", app.status);
    // The one read already, read again; or one read from the map.
    for keys in [
        &["<enter>", "<esc>", "\\", "\\", "<enter>"][..],
        &["M", "\\", "\\", "<enter>", "<enter>"],
    ] {
        let (app, _) = at(120, 30, keys);
        assert!(app.reading.is_some(), "{keys:?}");
        assert!(
            !app.status.contains("hidden until"),
            "{keys:?}: {}",
            app.status
        );
    }
}

#[test]
fn what_was_found_is_named_at_the_top_however_narrow() {
    let (_, screen) = at(70, 30, &["/", "grant", "<enter>"]);
    let top = screen.lines().next().unwrap();
    assert!(top.contains("grant"), "{top}");
    let (_, screen) = at(80, 30, &["/", "grant", "<enter>"]);
    let top = screen.lines().next().unwrap();
    assert!(top.contains("Found: grant"), "{top}");
}

#[test]
fn the_way_back_from_what_was_found_is_seen_60_wide() {
    let (_, screen) = at(60, 20, &["/", "Turing", "<enter>"]);
    assert!(screen.contains("esc goes back"), "{screen}");
}
