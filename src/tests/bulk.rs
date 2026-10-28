//! Several selected, acted on together only once asked, in the list: the
//! map is only looked at.
use super::mapsafety::{folders_of, message_id, trashed};
use super::mapscreen::demo_map;
use super::*;

/// The demo's list, its map read: what is selected is of the map's.
fn demo_list() -> App {
    let mut app = demo_map();
    press(&mut app, &["M"]);
    assert_eq!(app.map.mode, crate::map::Mode::List);
    app
}

/// The Message-IDs of `keys`, to tell them in Trash.
fn ids(app: &App, keys: &[crate::index::Key]) -> Vec<String> {
    keys.iter().map(|k| message_id(app, k)).collect()
}

/// The keys of the map's messages `keep` takes.
fn keys_where(app: &App, keep: impl Fn(&crate::map::Dot) -> bool) -> Vec<crate::index::Key> {
    let s = app.map.snapshot.as_ref().unwrap();
    s.dots
        .iter()
        .filter(|d| keep(d))
        .map(|d| d.key.clone())
        .collect()
}

fn filed(d: &crate::map::Dot) -> bool {
    !d.system.inbox && !d.system.sent && !d.system.draft
}

#[test]
fn several_marked_are_asked_about_before_they_are_trashed() {
    let mut app = demo_list();
    let keys = keys_where(&app, filed);
    assert!(keys.len() >= 3, "the demo files some mail");
    let keys: Vec<_> = keys.into_iter().take(3).collect();
    let ids = ids(&app, &keys);
    app.map.region = Some(crate::map::Region { keys: keys.clone() });
    press(&mut app, &["d"]);
    assert!(matches!(app.dialog, app::Dialog::Bulk(_)), "{}", app.status);
    let screen = app::screen(&mut app, 140, 40);
    assert!(screen.contains("3 to Trash"), "{screen}");
    // Anything but y leaves them, still marked.
    press(&mut app, &["<enter>"]);
    assert!(!app.dialog.is_open());
    assert_eq!(app.map.region.as_ref().map(|r| r.keys.len()), Some(3));
    assert!(ids.iter().all(|id| !trashed(&app, id)));
    // y moves them.
    press(&mut app, &["d", "y"]);
    if matches!(app.dialog, app::Dialog::Applied(_)) {
        press(&mut app, &["x"]);
    }
    assert!(ids.iter().all(|id| trashed(&app, id)), "{}", app.status);
}

#[test]
fn one_marked_is_trashed_at_once_as_d_always_did() {
    let mut app = demo_list();
    let key = keys_where(&app, filed).remove(0);
    let id = message_id(&app, &key);
    app.map.region = Some(crate::map::Region {
        keys: vec![key.clone()],
    });
    press(&mut app, &["d"]);
    assert!(!matches!(app.dialog, app::Dialog::Bulk(_)));
    if matches!(app.dialog, app::Dialog::Applied(_)) {
        press(&mut app, &["x"]);
    }
    assert!(trashed(&app, &id), "{}", app.status);
}

#[test]
fn inbox_and_sent_mail_are_left_out_of_a_bulk_trash_unless_taken() {
    let mut app = demo_list();
    let inbox = keys_where(&app, |d| d.system.inbox);
    let sent = keys_where(&app, |d| d.system.sent && !d.system.inbox);
    let other = keys_where(&app, filed);
    assert!(inbox.len() >= 2 && !sent.is_empty() && !other.is_empty());
    let (inbox, sent, other) = (inbox[..2].to_vec(), sent[..1].to_vec(), other[..1].to_vec());
    let (inbox_ids, sent_ids, other_ids) = (ids(&app, &inbox), ids(&app, &sent), ids(&app, &other));
    let all: Vec<_> = inbox.iter().chain(&sent).chain(&other).cloned().collect();
    app.map.region = Some(crate::map::Region { keys: all });
    press(&mut app, &["d"]);
    let screen = app::screen(&mut app, 140, 40);
    assert!(screen.contains("2 in an Inbox"), "{screen}");
    assert!(screen.contains("1 you sent"), "{screen}");
    // As asked: only what is neither.
    press(&mut app, &["y"]);
    if matches!(app.dialog, app::Dialog::Applied(_)) {
        press(&mut app, &["x"]);
    }
    assert!(trashed(&app, &other_ids[0]), "{}", app.status);
    assert!(
        inbox_ids
            .iter()
            .chain(&sent_ids)
            .all(|id| !trashed(&app, id))
    );

    // i takes the Inbox's too; s what was sent.
    let all: Vec<_> = inbox.iter().chain(&sent).cloned().collect();
    app.map.region = Some(crate::map::Region { keys: all });
    press(&mut app, &["d", "i"]);
    assert!(
        matches!(app.dialog, app::Dialog::Bulk(_)),
        "i keeps it open"
    );
    press(&mut app, &["y"]);
    if matches!(app.dialog, app::Dialog::Applied(_)) {
        press(&mut app, &["x"]);
    }
    assert!(
        inbox_ids.iter().all(|id| trashed(&app, id)),
        "{}",
        app.status
    );
    assert!(!trashed(&app, &sent_ids[0]));
}

#[test]
fn several_marked_are_asked_about_before_they_are_archived() {
    let mut app = demo_list();
    let inbox: Vec<_> = keys_where(&app, |d| d.system.inbox)
        .into_iter()
        .take(2)
        .collect();
    app.map.region = Some(crate::map::Region {
        keys: inbox.clone(),
    });
    press(&mut app, &["e"]);
    assert!(matches!(app.dialog, app::Dialog::Bulk(_)));
    let screen = app::screen(&mut app, 140, 40);
    assert!(screen.contains("Archive 2"), "{screen}");
    press(&mut app, &["y"]);
    let s = app.map.snapshot.clone().unwrap();
    for k in &inbox {
        let d = &s.dots[s.find(k).unwrap()];
        assert!(!d.system.inbox, "{}", d.subject);
    }
}

#[test]
fn what_is_marked_is_said_at_the_foot_while_it_is_marked() {
    let mut app = demo_list();
    let keys: Vec<_> = keys_where(&app, filed).into_iter().take(3).collect();
    app.map.region = Some(crate::map::Region { keys });
    app.status.clear();
    let screen = app::screen(&mut app, 140, 40);
    assert!(screen.contains("▪ 3 marked"), "{screen}");
}

#[test]
fn c_x_u_puts_back_what_was_last_trashed_together() {
    let mut app = demo_list();
    let keys: Vec<_> = keys_where(&app, filed).into_iter().take(3).collect();
    let ids = ids(&app, &keys);
    let from: Vec<Vec<String>> = keys.iter().map(|k| folders_of(&app, k)).collect();
    app.map.region = Some(crate::map::Region { keys: keys.clone() });
    press(&mut app, &["d", "y"]);
    if matches!(app.dialog, app::Dialog::Applied(_)) {
        press(&mut app, &["x"]);
    }
    assert!(ids.iter().all(|id| trashed(&app, id)), "{}", app.status);
    assert!(app.status.contains("C-x u"), "said how: {}", app.status);
    press(&mut app, &["<c-x>", "u"]);
    if matches!(app.dialog, app::Dialog::Applied(_)) {
        press(&mut app, &["x"]);
    }
    for ((k, id), was) in keys.iter().zip(&ids).zip(&from) {
        assert_eq!(&folders_of(&app, k), was, "{id} put back");
        assert!(!trashed(&app, id), "{id} out of the record");
    }
}

#[test]
fn trashing_many_together_teaches_nothing_and_one_alone_still_does() {
    let mut app = demo_list();
    let thrown = |app: &App| app.index.lock().unwrap().filter.thrown;
    let keys: Vec<_> = keys_where(&app, filed).into_iter().take(4).collect();
    let ids = ids(&app, &keys);
    // One alone: learnt, as d always taught.
    app.map.region = Some(crate::map::Region {
        keys: vec![keys[0].clone()],
    });
    press(&mut app, &["d"]);
    if matches!(app.dialog, app::Dialog::Applied(_)) {
        press(&mut app, &["x"]);
    }
    assert_eq!(thrown(&app), 1);
    // Three together: a cluster says nothing of each in it.
    app.map.region = Some(crate::map::Region {
        keys: keys[1..].to_vec(),
    });
    press(&mut app, &["d", "y"]);
    if matches!(app.dialog, app::Dialog::Applied(_)) {
        press(&mut app, &["x"]);
    }
    assert!(
        ids[1..].iter().all(|id| trashed(&app, id)),
        "{}",
        app.status
    );
    assert_eq!(thrown(&app), 1, "nothing learnt of the three");
}

#[test]
fn what_was_taught_between_two_times_is_forgotten() {
    let app = demo_list();
    let mut index = app.index.lock().unwrap();
    let keys: Vec<_> = index.records.keys().take(6).cloned().collect();
    for k in &keys[..2] {
        index.teach(k, false).unwrap();
    }
    for k in &keys[2..] {
        index.teach(k, true).unwrap();
    }
    // As if the last four were taught at 13:56, the rest before.
    let n = index.filter.taught.len();
    for (i, t) in index.filter.taught.iter_mut().enumerate() {
        t.at = if i + 4 >= n { 1000 } else { 10 };
    }
    let before = index.filter.thrown;
    assert_eq!(index.unteach(900, 1100).unwrap(), 4);
    assert_eq!(index.filter.thrown, before - 4);
    assert_eq!(index.filter.kept, 2);
    assert_eq!(index.filter.taught.len(), n - 4);
}

#[test]
fn c_x_u_takes_back_what_a_d_taught_and_teaches_nothing_new() {
    let mut app = demo_list();
    let lessons = |app: &App| {
        let index = app.index.lock().unwrap();
        (index.filter.thrown, index.filter.kept)
    };
    let before = lessons(&app);
    let key = keys_where(&app, filed).remove(0);
    let id = message_id(&app, &key);
    app.map.region = Some(crate::map::Region {
        keys: vec![key.clone()],
    });
    press(&mut app, &["d"]);
    if matches!(app.dialog, app::Dialog::Applied(_)) {
        press(&mut app, &["x"]);
    }
    assert!(trashed(&app, &id));
    assert_eq!(lessons(&app), (before.0 + 1, before.1), "d taught it");
    press(&mut app, &["<c-x>", "u"]);
    if matches!(app.dialog, app::Dialog::Applied(_)) {
        press(&mut app, &["x"]);
    }
    assert!(!trashed(&app, &id), "{}", app.status);
    assert_eq!(lessons(&app), before, "as if never taught");
}

#[test]
fn c_x_u_with_nothing_trashed_together_says_so() {
    let mut app = demo_list();
    press(&mut app, &["<c-x>", "u"]);
    assert!(app.status.contains("nothing"), "{}", app.status);
}
