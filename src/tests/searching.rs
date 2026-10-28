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
