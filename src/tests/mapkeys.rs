//! The keys on the map and in the Inbox looked through beside it: what
//! each does there, and what the foot and the lines about it say.

use super::*;

/// The foot's last lines, on a screen so wide.
fn foot_of(app: &mut App, width: u16, height: u16) -> String {
    let screen = app::screen(app, width, height);
    let lines: Vec<&str> = screen.lines().collect();
    lines[lines.len().saturating_sub(3)..].join("\n")
}

#[test]
fn space_pages_the_message_read_beside_the_map() {
    let mut app = demo_map();
    press(&mut app, &["<enter>", "<enter>"]);
    assert!(app.reading.is_some(), "one read beside the map");
    assert_eq!(app.focus, Focus::List, "the map keeps the focus");
    // Small enough that it does not fit.
    let small = |app: &mut App, key: &str| {
        let _ = app::screen(app, 120, 35);
        app.key(app::keys(key)[0]);
        app.settle(Duration::from_secs(5));
    };
    let before = app.scroll;
    small(&mut app, "<space>");
    assert!(app.scroll > before, "paged: {} after {before}", app.scroll);
    small(&mut app, "b");
    assert_eq!(app.scroll, before, "and back");
}

#[test]
fn the_maps_keys_from_beside_it_say_tab_goes_to_it() {
    let mut app = demo_map();
    press(&mut app, &["i", "z"]);
    assert!(
        app.status.contains("tab goes to it"),
        "the map is on screen: {}",
        app.status
    );
    // The view switched, what was said of the other is not said.
    press(&mut app, &["M"]);
    assert!(!app.status.contains("map's"), "{}", app.status);
}

#[test]
fn y_capital_in_the_inbox_looked_through_chooses_all_like_it() {
    let mut app = demo_map();
    press(&mut app, &["i"]);
    let (filed, discarded, kept, _) = app.review_counts();
    assert_eq!(filed + discarded + kept, 0);
    press(&mut app, &["Y"]);
    let (filed, discarded, kept, _) = app.review_counts();
    assert!(filed + discarded + kept > 1, "more than the one picked");
    assert!(app.status.contains("C-c C-c does it"), "{}", app.status);
    // Taken back together.
    press(&mut app, &["<c-/>"]);
    let (filed, discarded, kept, _) = app.review_counts();
    assert_eq!(filed + discarded + kept, 0, "{}", app.status);
}

#[test]
fn the_foot_on_the_clusters_offers_no_key_refused_there() {
    let mut app = demo_map();
    let foot = foot_of(&mut app, 170, 45);
    for refused in [" d trash", " e archive", " m move to"] {
        assert!(!foot.contains(refused), "{refused}: {foot}");
    }
    assert!(foot.contains("⏎ read"), "{foot}");
    // Down at a message, they act, and are offered.
    press(&mut app, &["<enter>"]);
    let foot = foot_of(&mut app, 170, 45);
    for offered in [" d trash", " e archive", " m move to"] {
        assert!(foot.contains(offered), "{offered}: {foot}");
    }
}

#[test]
fn the_foot_reading_one_looked_through_says_what_the_keys_do_there() {
    let mut app = demo_map();
    press(&mut app, &["i", "<enter>"]);
    let foot = foot_of(&mut app, 170, 45);
    assert!(foot.contains("d discard"), "{foot}");
    assert!(foot.contains("y/Y as suggested"), "{foot}");
    assert!(!foot.contains("trash"), "{foot}");
    assert!(!foot.contains("archive"), "{foot}");
}

#[test]
fn the_map_searched_shows_no_count_of_the_list() {
    let mut app = demo_map();
    press(&mut app, &["/", "grant", "<enter>"]);
    let screen = app::screen(&mut app, 170, 45);
    let mut lines = screen.lines();
    let top = lines.next().unwrap();
    assert!(!top.contains("messages"), "{top}");
    assert!(lines.next().unwrap().contains("found for"), "{screen}");
}

#[test]
fn the_cluster_narrowed_to_is_said_first() {
    let mut app = demo_map();
    press(&mut app, &["]"]);
    let screen = app::screen(&mut app, 90, 30);
    let title = screen.lines().nth(1).unwrap();
    assert!(title.starts_with(" Map · in "), "{title}");
}

#[test]
fn m_in_a_clusters_tree_goes_out_to_the_clusters() {
    let mut app = demo_map();
    press(&mut app, &["<enter>"]);
    assert!(app.map.tree.is_some());
    press(&mut app, &["m"]);
    assert!(app.map.moving.is_some(), "{}", app.status);
    assert!(app.map.tree.is_none(), "out to the clusters, to pick one");
}

#[test]
fn undo_takes_back_a_choice_not_yet_done() {
    let mut app = demo_map();
    press(&mut app, &["i", "<space>", "d"]);
    assert_eq!(app.review_counts().1, 1);
    assert_eq!(app.review_counts().2, 1);
    press(&mut app, &["<c-/>"]);
    assert_eq!(app.review_counts().1, 0, "{}", app.status);
    assert_eq!(app.review_counts().2, 1);
    press(&mut app, &["<c-x>", "u"]);
    assert_eq!(app.review_counts().2, 0, "{}", app.status);
    // With none, what was done, as anywhere.
    press(&mut app, &["<c-/>"]);
    assert!(app.status.contains("put back"), "{}", app.status);
}

#[test]
fn m_r_is_not_offered_again_when_it_came_out_no_nearer() {
    let mut app = demo_map();
    let screen = app::screen(&mut app, 170, 45);
    assert!(screen.contains("M-r lays it out again"), "{screen}");
    press(&mut app, &["<m-r>"]);
    assert!(app.status.contains("no nearer"), "{}", app.status);
    press(&mut app, &["<enter>"]);
    let screen = app::screen(&mut app, 170, 45);
    assert!(!screen.contains("M-r lays it out again"), "{screen}");
    assert!(screen.contains("laid out again, no nearer"), "{screen}");
}

#[test]
fn a_word_is_cut_from_what_is_typed_in_a_prompt() {
    let mut app = demo_map();
    press(&mut app, &["k", "k", "R"]);
    let app::Dialog::RenameLabel(old, _) = &app.dialog else {
        panic!("asked: {:?}", app.status);
    };
    let old = old.clone();
    press(&mut app, &["<c-w>", "Big Funding"]);
    app.key(KeyEvent::new(KeyCode::Backspace, KeyModifiers::ALT));
    press(&mut app, &["Fund"]);
    let app::Dialog::RenameLabel(_, typed) = &app.dialog else {
        panic!("still asked");
    };
    assert_eq!(typed, "Big Fund", "{old} cut, then a word");
    let screen = app::screen(&mut app, 170, 45);
    assert!(screen.contains("C-u clears"), "{screen}");
}

#[test]
fn the_inbox_looked_through_says_its_hint_in_whole_words() {
    let mut app = demo_map();
    press(&mut app, &["i"]);
    let screen = app::screen(&mut app, 170, 45);
    let hint = screen
        .lines()
        .find_map(|l| l.split_once(" none chosen").map(|(_, h)| h.trim_end()))
        .unwrap_or_else(|| panic!("{screen}"));
    assert!(
        hint.is_empty() || hint.ends_with("those") || hint.ends_with("chosen"),
        "{hint}"
    );
}
