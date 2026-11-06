//! C-x C-s: what is being written kept in Drafts, and gone on with; saved
//! again it replaces what was saved, and sent, what was saved goes.
use super::*;

/// The demo, with a store to look at it by.
fn writing_with_store() -> (App, Box<dyn Store>) {
    let demo = Demo::new();
    let store = demo.twin().unwrap();
    let mut app = App::new(
        vec![("Demo".to_string(), Box::new(demo) as Box<dyn Store>)],
        PathBuf::from("/nowhere/accounts.toml"),
    );
    app.ledger = scratch_ledger();
    app.send_hold = Duration::ZERO;
    app.settle(PATIENCE);
    (app, store)
}

fn subjects(store: &mut Box<dyn Store>, folder: &str) -> Vec<String> {
    store
        .page(folder, 0, 200)
        .unwrap()
        .summaries
        .into_iter()
        .map(|s| s.subject)
        .collect()
}

fn count(store: &mut Box<dyn Store>, folder: &str, subject: &str) -> usize {
    subjects(store, folder)
        .iter()
        .filter(|s| *s == subject)
        .count()
}

/// A new message, its subject and what is written given.
fn begun(app: &mut App, subject: &str, body: &str) {
    press(
        app,
        &[
            "c",
            "ada@analytical.example",
            "<tab>",
            "<tab>",
            subject,
            "<tab>",
            body,
        ],
    );
}

fn save(app: &mut App) {
    press(app, &["<c-x>", "<c-s>"]);
    app.settle(PATIENCE);
}

#[test]
fn c_x_c_s_keeps_it_in_drafts_and_goes_on_writing() {
    let (mut app, mut store) = writing_with_store();
    begun(&mut app, "Kept", "First words");
    save(&mut app);
    assert!(app.compose.is_some(), "still writing: {}", app.status);
    assert!(app.status.contains("saved in Drafts"), "{}", app.status);
    assert_eq!(count(&mut store, "Drafts", "Kept"), 1);
    assert_eq!(count(&mut store, "Sent", "Kept"), 0, "nothing sent");
}

#[test]
fn saved_again_it_replaces_what_was_saved_which_is_kept_in_trash() {
    let (mut app, mut store) = writing_with_store();
    begun(&mut app, "Twice", "First words");
    save(&mut app);
    press(&mut app, &[" and more"]);
    save(&mut app);
    app.settle(PATIENCE);
    assert_eq!(count(&mut store, "Drafts", "Twice"), 1, "{}", app.status);
    assert_eq!(count(&mut store, "Trash", "Twice"), 1, "not gone for good");
    let drafts = store.page("Drafts", 0, 200).unwrap().summaries;
    let kept = drafts.iter().find(|s| s.subject == "Twice").unwrap();
    let raw = store.raw("Drafts", kept.uid).unwrap();
    assert!(
        String::from_utf8_lossy(&raw).contains("First words and more"),
        "the newer"
    );
}

#[test]
fn sent_what_was_saved_goes_and_is_not_left_to_send_twice() {
    let (mut app, mut store) = writing_with_store();
    begun(&mut app, "Off", "First words");
    save(&mut app);
    press(&mut app, &["<c-c>", "<c-c>"]);
    app.settle(PATIENCE);
    assert!(app.compose.is_none(), "{}", app.status);
    assert_eq!(count(&mut store, "Sent", "Off"), 1);
    assert_eq!(count(&mut store, "Drafts", "Off"), 0, "{}", app.status);
}

#[test]
fn a_letter_to_no_one_yet_may_be_saved() {
    let (mut app, mut store) = writing_with_store();
    press(&mut app, &["c", "<tab>", "<tab>", "Unaddressed"]);
    save(&mut app);
    assert!(app.status.contains("saved in Drafts"), "{}", app.status);
    assert_eq!(count(&mut store, "Drafts", "Unaddressed"), 1);
}

#[test]
fn saved_and_unchanged_it_is_let_go_unasked_and_stays_a_draft() {
    let (mut app, mut store) = writing_with_store();
    begun(&mut app, "Left", "First words");
    save(&mut app);
    press(&mut app, &["<c-c>", "<c-k>"]);
    assert!(app.compose.is_none(), "not asked: {}", app.status);
    assert_eq!(count(&mut store, "Drafts", "Left"), 1);
}

#[test]
fn what_could_not_be_saved_says_so_and_is_still_being_written() {
    let demo = Demo::new();
    demo.refuse("Refused");
    let store = demo.twin().unwrap();
    let mut store = store;
    let mut app = App::new(
        vec![("Demo".to_string(), Box::new(demo) as Box<dyn Store>)],
        PathBuf::from("/nowhere/accounts.toml"),
    );
    app.ledger = scratch_ledger();
    app.settle(PATIENCE);
    begun(&mut app, "Refused", "First words");
    save(&mut app);
    assert!(app.compose.is_some());
    assert!(app.status.contains("not saved"), "{}", app.status);
    assert_eq!(count(&mut store, "Drafts", "Refused"), 0);
}

#[test]
fn a_reply_saved_goes_on_answering_what_it_answers() {
    let (mut app, mut store) = writing_with_store();
    press(&mut app, &["r"]);
    app.settle(PATIENCE);
    assert!(app.compose.is_some(), "{}", app.status);
    save(&mut app);
    let drafts = store.page("Drafts", 0, 200).unwrap().summaries;
    let kept = drafts.iter().max_by_key(|s| s.uid).unwrap();
    let raw = store.raw("Drafts", kept.uid).unwrap();
    assert!(
        String::from_utf8_lossy(&raw).contains("In-Reply-To:"),
        "{}",
        String::from_utf8_lossy(&raw)
    );
}

#[test]
fn saving_a_draft_teaches_nothing() {
    let (mut app, _store) = writing_with_store();
    let lessons = |app: &App| {
        let index = app.index.lock().unwrap();
        (index.filter.thrown, index.filter.kept)
    };
    let before = lessons(&app);
    begun(&mut app, "Quiet", "First words");
    save(&mut app);
    press(&mut app, &[" more"]);
    save(&mut app);
    app.settle(PATIENCE);
    assert_eq!(lessons(&app), before);
}

fn fixing() -> App {
    let mut app = demo();
    app.grammar_open = |_| {
        Ok(Box::new(crate::grammar::Using(|s: &str| {
            Ok(s.replace("you time", "your time"))
        })))
    };
    app
}

#[test]
fn the_grammar_is_c_c_g_and_the_buttons_say_so() {
    let mut app = fixing();
    begun(&mut app, "Words", "Thank you for you time.");
    let screen = app::screen(&mut app, 210, 40);
    assert!(screen.contains("Grammar  C-c g"), "{screen}");
    assert!(screen.contains("Save  C-x C-s"), "{screen}");
    press(&mut app, &["<c-c>", "g"]);
    app.settle(PATIENCE);
    assert!(app.status.contains("“you” → “your”"), "{}", app.status);
}

#[test]
fn c_c_c_g_lets_c_c_go_and_reads_no_model() {
    use std::sync::atomic::{AtomicBool, Ordering};
    static READ: AtomicBool = AtomicBool::new(false);
    let mut app = demo();
    app.grammar_open = |_| {
        READ.store(true, Ordering::SeqCst);
        anyhow::bail!("not to be read")
    };
    begun(&mut app, "Words", "Thank you.");
    press(&mut app, &["<c-c>", "<c-g>"]);
    app.settle(Duration::from_millis(200));
    assert!(app.compose.is_some());
    assert!(!READ.load(Ordering::SeqCst), "{}", app.status);
    assert!(!app.status.contains("checking"), "{}", app.status);
}

#[test]
fn asked_to_send_as_a_check_comes_back_n_skips_the_change_and_sends_nothing() {
    let mut app = demo();
    // Slow enough for C-c C-c to come while it is being read.
    app.grammar_open = |_| {
        std::thread::sleep(Duration::from_millis(300));
        Ok(Box::new(crate::grammar::Using(|s: &str| {
            Ok(s.replace("you time", "your time"))
        })))
    };
    app.check_on_send = true;
    begun(&mut app, "Words", "Thank you for you time.");
    press_unsettled(&mut app, &["<c-c>", "g", "<c-c>", "<c-c>"]);
    assert!(app.check_asked, "{}", app.status);
    app.settle(PATIENCE);
    assert!(app.status.contains("“you” → “your”"), "{}", app.status);
    press(&mut app, &["n"]);
    let compose = app.compose.as_ref().expect("still writing");
    assert!(compose.held.is_none(), "not sent: {}", app.status);
}

#[test]
fn a_save_answered_late_is_not_taken_for_the_letter_written_since() {
    let (mut app, mut store) = writing_with_store();
    begun(&mut app, "Letter A", "First words");
    press_unsettled(&mut app, &["<c-x>", "<c-s>", "<c-c>", "<c-k>", "y"]);
    assert!(app.compose.is_none(), "{}", app.status);
    press_unsettled(
        &mut app,
        &[
            "c",
            "ada@analytical.example",
            "<tab>",
            "<tab>",
            "Letter B",
            "<tab>",
            "Other words",
        ],
    );
    press_unsettled(&mut app, &["<c-x>", "<c-s>"]);
    app.settle(PATIENCE);
    press(&mut app, &["<c-c>", "<c-c>"]);
    app.settle(PATIENCE);
    assert_eq!(count(&mut store, "Sent", "Letter B"), 1, "{}", app.status);
    assert_eq!(
        count(&mut store, "Drafts", "Letter B"),
        0,
        "its own draft went"
    );
    assert_eq!(
        count(&mut store, "Drafts", "Letter A"),
        1,
        "the other's stays"
    );
}

#[test]
fn saved_and_unchanged_it_is_not_saved_again() {
    let (mut app, mut store) = writing_with_store();
    begun(&mut app, "Once", "First words");
    save(&mut app);
    save(&mut app);
    assert!(app.status.contains("already"), "{}", app.status);
    assert_eq!(count(&mut store, "Drafts", "Once"), 1);
    assert_eq!(count(&mut store, "Trash", "Once"), 0, "nothing replaced");
}

#[test]
fn a_file_that_cannot_be_read_is_said_and_nothing_is_saved() {
    let (mut app, mut store) = writing_with_store();
    begun(&mut app, "Missing", "First words");
    let text = app.compose.as_ref().unwrap().buffer.text();
    let with = text.replacen("\n\n", "\nAttach: /nowhere/at/all.pdf\n\n", 1);
    app.compose.as_mut().unwrap().buffer = crate::edit::Buffer::from_text(&with);
    save(&mut app);
    assert!(app.status.contains("not saved"), "{}", app.status);
    assert!(app.compose.is_some());
    assert_eq!(count(&mut store, "Drafts", "Missing"), 0);
}

#[test]
fn c_c_c_s_does_nothing_now() {
    let mut app = demo();
    begun(&mut app, "Words", "Some words.");
    let before = app.compose.as_ref().unwrap().buffer.text();
    press(&mut app, &["<c-c>", "<c-s>"]);
    assert!(app.status.contains("undefined"), "{}", app.status);
    assert_eq!(app.compose.as_ref().unwrap().buffer.text(), before);
}
