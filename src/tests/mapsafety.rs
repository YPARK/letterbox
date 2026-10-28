//! The map is looked at: on its clusters nothing moves mail; down at a
//! message in a cluster's tree, or reading one, it is acted on alone.
use super::mapscreen::demo_map;
use super::*;

/// The folders the message `key` is in now, as the index has them: Trash
/// too, which the map leaves out.
pub(super) fn folders_of(app: &App, key: &crate::index::Key) -> Vec<String> {
    let index = app.index.lock().unwrap();
    index
        .records
        .get(key)
        .map(|r| r.places.iter().map(|p| p.folder.clone()).collect())
        .unwrap_or_default()
}

/// The Message-ID of the message `key`.
pub(super) fn message_id(app: &App, key: &crate::index::Key) -> String {
    app.index.lock().unwrap().records[key].message_id.clone()
}

/// Whether the message of Message-ID `id` is in Trash, as its server
/// confirmed and letterbox recorded, to be put back.
pub(super) fn trashed(app: &App, id: &str) -> bool {
    app.ledger.find("you@example.com", id).is_some()
}

/// Down the cluster's tree to the message `key`, as ⏎ and the arrows go.
pub(super) fn down_to(app: &mut App, key: &crate::index::Key) {
    let id = {
        let s = app.map.snapshot.as_ref().unwrap();
        s.group_of(s.find(key).unwrap()).unwrap().id
    };
    app.map.cursor = Some(key.clone());
    app.map.open_block(id);
    app.map.cursor = Some(key.clone());
    let now = jiff::Timestamp::now().as_second();
    while app.map.tree_in(now) {}
    assert!(app.map.at_message(now), "down at the message");
}

/// A message of the demo filed under a label, out of the Inbox.
fn a_filed(app: &App) -> crate::index::Key {
    let s = app.map.snapshot.as_ref().unwrap();
    s.dots
        .iter()
        .find(|d| !d.system.inbox && !d.system.sent && !d.labels.is_empty())
        .unwrap()
        .key
        .clone()
}

/// An unread message of the demo in the Inbox, filed nowhere.
fn an_inbox(app: &App) -> crate::index::Key {
    let s = app.map.snapshot.as_ref().unwrap();
    s.dots
        .iter()
        .find(|d| d.system.inbox && !d.system.sent)
        .unwrap()
        .key
        .clone()
}

#[test]
fn on_the_clusters_no_key_moves_mail() {
    let mut app = demo_map();
    for key in [a_filed(&app), an_inbox(&app)] {
        app.map.cursor = Some(key.clone());
        assert!(app.map.tree.is_none());
        let was = folders_of(&app, &key);
        for k in ["d", "e", "m", "y", "Y", "-", "N", "v"] {
            app.status.clear();
            press(&mut app, &[k]);
            assert!(
                !app.dialog.is_open() && app.map.moving.is_none() && app.map.region.is_none(),
                "{k}: {}",
                app.status
            );
            assert!(app.status.contains("looked at"), "{k}: {}", app.status);
            press(&mut app, &["<esc>"]);
            app.map.cursor = Some(key.clone());
        }
        assert_eq!(folders_of(&app, &key), was, "nothing moved");
    }
}

#[test]
fn down_at_a_message_in_a_clusters_tree_d_trashes_it_alone() {
    let mut app = demo_map();
    let key = a_filed(&app);
    let id = message_id(&app, &key);
    down_to(&mut app, &key);
    press(&mut app, &["d"]);
    if matches!(app.dialog, app::Dialog::Applied(_)) {
        press(&mut app, &["x"]);
    }
    assert!(trashed(&app, &id), "{}", app.status);
}

#[test]
fn reading_on_the_map_d_trashes_the_message_read_not_the_lists() {
    let mut app = demo_map();
    let key = a_filed(&app);
    let id = message_id(&app, &key);
    let listed = app.current().map(|l| (l.folder.clone(), l.summary.uid));
    down_to(&mut app, &key);
    press(&mut app, &["<enter>"]);
    assert!(app.reading.is_some(), "{}", app.status);
    app.focus = Focus::Reader;
    press(&mut app, &["d"]);
    if matches!(app.dialog, app::Dialog::Applied(_)) {
        press(&mut app, &["x"]);
    }
    assert!(trashed(&app, &id), "the message read: {}", app.status);
    // The list's behind the map, untouched.
    let (folder, uid) = listed.expect("the list has one picked");
    assert!(
        app.shown
            .iter()
            .any(|l| l.folder == folder && l.summary.uid == uid),
        "the list's message is where it was"
    );
}
