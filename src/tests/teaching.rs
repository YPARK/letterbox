//! What the filter learns from the Inbox looked through: each message
//! decided alone, never what was chosen for several together, nor the
//! filter's own guess taken as it is.
use super::*;

fn thrown(app: &App) -> u32 {
    app.index.lock().unwrap().filter.thrown
}

fn kept(app: &App) -> u32 {
    app.index.lock().unwrap().filter.kept
}

/// $ and y: what was chosen, done.
fn apply(app: &mut App) {
    press(app, &["<c-c>", "<c-c>"]);
    assert!(
        matches!(app.dialog, app::Dialog::Apply(_)),
        "{}",
        app.status
    );
    press(app, &["y"]);
    app.settle(Duration::from_secs(5));
    if matches!(app.dialog, app::Dialog::Applied(_)) {
        press(app, &["x"]);
    }
}

/// The row of the Inbox looked through whose message `keep` takes.
fn row_where(app: &App, keep: impl Fn(&crate::map::Dot) -> bool) -> usize {
    let s = app.map.snapshot.as_ref().unwrap();
    app.review_rows()
        .iter()
        .position(|&i| keep(&s.dots[i]))
        .expect("such a row")
}

#[test]
fn one_discarded_alone_is_learnt() {
    let mut app = super::mapscreen::demo_map();
    press(&mut app, &["i"]);
    let before = thrown(&app);
    press(&mut app, &["d"]);
    apply(&mut app);
    assert_eq!(thrown(&app), before + 1);
}

#[test]
fn what_is_chosen_for_several_together_is_not_learnt() {
    let mut app = super::mapscreen::demo_map();
    press(&mut app, &["i"]);
    let (t, k) = (thrown(&app), kept(&app));
    super::selecting::drag(
        &mut app,
        app::Target::ReviewRow(0),
        app::Target::ReviewRow(2),
    );
    press(&mut app, &["d"]);
    super::selecting::drag(
        &mut app,
        app::Target::ReviewRow(3),
        app::Target::ReviewRow(4),
    );
    press(&mut app, &[" "]);
    apply(&mut app);
    assert_eq!((thrown(&app), kept(&app)), (t, k), "{}", app.status);
}

#[test]
fn the_filters_own_guess_taken_as_it_is_is_not_learnt() {
    let mut app = demo_taught();
    press(&mut app, &["i"]);
    let row = row_where(&app, |d| d.thrown.is_some());
    for _ in 0..row {
        press(&mut app, &["<down>"]);
    }
    let before = thrown(&app);
    // y takes what is suggested: discard, as the filter guessed.
    press(&mut app, &["y"]);
    apply(&mut app);
    assert_eq!(thrown(&app), before, "its own guess, not learnt");
}

#[test]
fn y_capital_leaves_what_was_chosen_already_as_it_was() {
    let mut app = super::mapscreen::demo_map();
    press(&mut app, &["i"]);
    let s = app.map.snapshot.clone().unwrap();
    let rows = app.review_rows();
    // Two rows suggested alike, to be filed.
    let (a, b) = (0..rows.len())
        .flat_map(|a| (a + 1..rows.len()).map(move |b| (a, b)))
        .find(|&(a, b)| {
            let like = App::suggested(&s, rows[a]);
            matches!(like, app::Choice::Move(_)) && like == App::suggested(&s, rows[b])
        })
        .expect("two suggested alike");
    for _ in 0..a {
        press(&mut app, &["<down>"]);
    }
    press(&mut app, &["d"]);
    for _ in a + 1..b {
        press(&mut app, &["<down>"]);
    }
    press(&mut app, &["Y"]);
    assert_eq!(
        app.chosen(rows[a]),
        Some(app::Choice::Discard),
        "kept as chosen"
    );
    assert_eq!(app.chosen(rows[b]), Some(App::suggested(&s, rows[b])));
}

#[test]
fn a_choice_taken_back_is_learnt_from_as_it_was_before() {
    let mut app = super::mapscreen::demo_map();
    press(&mut app, &["i"]);
    let (t, k) = (thrown(&app), kept(&app));
    super::selecting::drag(
        &mut app,
        app::Target::ReviewRow(0),
        app::Target::ReviewRow(2),
    );
    press(&mut app, &["d"]);
    // The first kept alone, then taken back: discarded with the rest,
    // and not learnt, as it was.
    press(&mut app, &["<up>", "<up>", "<up>"]);
    let first = app.review_rows()[0];
    assert_eq!(app.review_at().map(|(i, _)| i), Some(first));
    press(&mut app, &[" "]);
    assert_eq!(app.chosen(first), Some(app::Choice::Keep));
    press(&mut app, &["<c-/>"]);
    assert_eq!(
        app.chosen(first),
        Some(app::Choice::Discard),
        "{}",
        app.status
    );
    apply(&mut app);
    assert_eq!((thrown(&app), kept(&app)), (t, k), "{}", app.status);
}
