//! Search, of every folder of every account at once, from what letterbox
//! already knows: as fast as typing, and never only the folder listed.
use super::mapscreen::demo_map;
use super::*;

/// The subjects listed.
fn listed(app: &App) -> Vec<String> {
    app.shown
        .iter()
        .map(|l| l.summary.subject.clone())
        .collect()
}

/// The demo's list of the Inbox, its map read.
fn inbox() -> App {
    let mut app = demo_map();
    press(&mut app, &["M"]);
    assert_eq!(app.folder_name(), "Inbox");
    app
}

#[test]
fn enter_searches_every_folder_from_what_is_known() {
    let mut app = inbox();
    // In Archive, not the Inbox listed.
    assert!(!listed(&app).iter().any(|s| s.contains("ATAC peaks")));
    press(&mut app, &["/", "chr7", "<enter>"]);
    assert!(
        listed(&app)
            .iter()
            .any(|s| s.contains("ATAC peaks look odd")),
        "{:?}: {}",
        listed(&app),
        app.status
    );
    assert!(app.folder_name().contains("chr7"), "{}", app.folder_name());
    assert!(app.status.contains("every folder"), "{}", app.status);
}

#[test]
fn every_word_must_be_there_and_case_is_let_go() {
    let mut app = inbox();
    press(&mut app, &["/", "PHOTO 51", "<enter>"]);
    let found = listed(&app);
    assert!(!found.is_empty(), "{}", app.status);
    assert!(
        found.iter().all(|s| s.to_lowercase().contains("photo 51")),
        "{found:?}"
    );
    // Mail you sent too, as the map has it.
    assert!(app.shown.iter().any(|l| l.from == "You"), "{found:?}");
}

#[test]
fn nothing_found_is_said_and_esc_goes_back() {
    let mut app = inbox();
    press(&mut app, &["/", "zzqqxx", "<enter>"]);
    assert!(listed(&app).is_empty());
    assert!(app.status.contains("nothing found"), "{}", app.status);
    // esc: back to what was listed before.
    press(&mut app, &["<esc>"]);
    assert_eq!(app.folder_name(), "Inbox");
    assert!(!listed(&app).is_empty());
}

/// The map narrowed to the cluster labelled `label`, by ] as it is done.
fn into_cluster(app: &mut App, label: &str) -> u32 {
    let s = app.map.snapshot.clone().unwrap();
    let id = s
        .groups
        .iter()
        .find(|g| g.label.as_deref() == Some(label))
        .unwrap()
        .id;
    for _ in 0..s.groups.len() {
        if app.map.focused == crate::map::Focused::Cluster(id) {
            break;
        }
        press(app, &["]"]);
    }
    assert_eq!(app.map.focused, crate::map::Focused::Cluster(id));
    id
}

#[test]
fn on_the_map_a_search_in_a_cluster_is_of_that_cluster_only() {
    let mut app = demo_map();
    // Of all of the map, more than the Grant cluster holds.
    let now = jiff::Timestamp::now().as_second();
    press(&mut app, &["/", "review", "<enter>"]);
    let everywhere = app.map.found("review", now).len();
    press(&mut app, &["<esc>"]);

    let grant = into_cluster(&mut app, "Grant");
    press(&mut app, &["/", "review", "<enter>"]);
    let s = app.map.snapshot.clone().unwrap();
    let found = app.map.found("review", now);
    assert!(!found.is_empty(), "{}", app.status);
    assert!(found.len() < everywhere, "{} of {everywhere}", found.len());
    assert!(found.iter().all(|&i| s.picked_by(grant, i)));
    assert!(app.status.contains("in Grant"), "{}", app.status);
    let screen = app::screen(&mut app, 150, 45);
    assert!(
        screen.contains(&format!("{} found in Grant", found.len())),
        "{screen}"
    );
    // n goes round those of the cluster, never out of it.
    for _ in 0..=found.len() {
        press(&mut app, &["n"]);
        assert!(s.picked_by(grant, app.map.picked().unwrap()));
    }
    // 0: all of the map, and searched all again.
    press(&mut app, &["<esc>", "0"]);
    assert_eq!(app.map.focused, crate::map::Focused::All);
    press(&mut app, &["/", "review", "<enter>"]);
    assert_eq!(app.map.found("review", now).len(), everywhere);
}

/// The cluster labelled `label`, listed from its row in the sidebar.
fn list_cluster(app: &mut App, label: &str) -> u32 {
    let s = app.map.snapshot.clone().unwrap();
    let id = s
        .groups
        .iter()
        .find(|g| g.label.as_deref() == Some(label))
        .unwrap()
        .id;
    app.focus = app::Focus::Side;
    app.row = app
        .rows
        .iter()
        .position(|r| *r == app::Row::Cluster(id))
        .expect("its row");
    press(app, &["<enter>"]);
    assert_eq!(app.folder_name(), label);
    id
}

#[test]
fn in_the_list_a_search_of_a_cluster_listed_is_of_that_cluster_only() {
    let mut app = inbox();
    press(&mut app, &["/", "review", "<enter>"]);
    let everywhere = app.shown.len();
    press(&mut app, &["<esc>"]);

    let grant = list_cluster(&mut app, "Grant");
    let s = app.map.snapshot.clone().unwrap();
    press(&mut app, &["/", "review", "<enter>"]);
    let found = app.shown.len();
    assert!(found > 0, "{}", app.status);
    assert!(found < everywhere, "{found} of {everywhere}");
    assert!(
        app.shown.iter().all(|l| s
            .dots
            .iter()
            .enumerate()
            .any(|(i, d)| d.subject == l.summary.subject && s.picked_by(grant, i))),
        "{:?}",
        listed(&app)
    );
    assert!(
        app.folder_name().contains("in Grant"),
        "{}",
        app.folder_name()
    );
    assert!(app.status.contains("in Grant"), "{}", app.status);
    // A search again, of what was found there, is still of Grant.
    press(&mut app, &["/", "<c-u>", "reviewer", "<enter>"]);
    assert!(
        app.folder_name().contains("in Grant"),
        "{}",
        app.folder_name()
    );
    // esc: Grant listed again, as it was before the search.
    press(&mut app, &["<esc>"]);
    assert_eq!(app.folder_name(), "Grant");
}
