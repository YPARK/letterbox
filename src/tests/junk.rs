//! What is thrown away as junk, as the filter is sure: said, never done
//! unasked; listed by J, each chosen; and told of, by B and as letterbox
//! opens.

use super::*;

// ---- thrown away on its own, and told of ----

/// An index taught that the shop's bulk mail is thrown away and Ada's
/// kept, the shop's next deal in it.
pub(super) fn taught_index(
    dir: &std::path::Path,
    id: &age::x25519::Identity,
) -> (Index, index::Record) {
    let mut index = Index::open_with(dir, id);
    let mut records = Vec::new();
    for k in 0..30 {
        records.push((
            with_headers(
                &format!("s{k}"),
                "Shop <no-reply@shop.example>",
                &format!("Offer {k}"),
                "List-Unsubscribe: <mailto:out@x>\r\n",
            ),
            true,
        ));
        records.push((
            with_headers(
                &format!("a{k}"),
                "ada@analytical.example",
                &format!("Engine {k}"),
                "",
            ),
            false,
        ));
    }
    let deal = with_headers(
        "next",
        "Shop <no-reply@shop.example>",
        "Last chance on the offer",
        "List-Unsubscribe: <mailto:out@x>\r\n",
    );
    index
        .cache(
            records
                .iter()
                .map(|(r, _)| r)
                .chain([&deal])
                .map(|r| Cached::Upsert(Box::new(r.clone())))
                .collect(),
        )
        .unwrap();
    for (r, thrown) in &records {
        index.teach(&r.key, *thrown).unwrap();
    }
    (index, deal)
}

#[test]
fn what_is_thrown_away_on_its_own_is_written_down_and_never_learnt() {
    let dir = tempfile::tempdir().unwrap();
    let id = identity();
    let (mut index, deal) = taught_index(dir.path(), &id);
    let known = index.known();
    let tokens = crate::filter::Filter::tokens(&deal, &known);
    let (odds, counts) = (
        index.filter.odds(&tokens),
        (index.filter.thrown, index.filter.kept),
    );
    let graded = crate::filter::Filter::check(&index.filter.taught).len();
    index.throw_on_its_own(&deal.key).unwrap();
    // Written down, with what tells it, but not learnt from.
    let auto = index.filter.taught.last().unwrap().clone();
    assert!(auto.auto && auto.thrown);
    assert_eq!(auto.subject, "Last chance on the offer");
    assert_eq!(auto.from, "Shop");
    assert!(auto.odds >= 0.95, "{}", auto.odds);
    assert!(auto.at > 0);
    assert_eq!(index.filter.odds(&tokens), odds);
    assert_eq!((index.filter.thrown, index.filter.kept), counts);
    assert_eq!(
        crate::filter::Filter::check(&index.filter.taught).len(),
        graded
    );
    // So after a start too.
    index.flush().unwrap();
    drop(index);
    let mut index = Index::open_with(dir.path(), &id);
    assert_eq!(index.filter.odds(&tokens), odds);
    // Put back, it was wrong: learnt as kept.
    assert!(index.put_back(&deal.key.account, &deal.message_id).unwrap());
    assert_eq!(index.filter.kept, counts.1 + 1);
    assert!(index.filter.odds(&tokens) < odds);
}

#[test]
fn why_it_would_be_thrown_away_is_said_in_words() {
    let dir = tempfile::tempdir().unwrap();
    let (index, deal) = taught_index(dir.path(), &identity());
    let why = index
        .filter
        .why(&crate::filter::Filter::tokens(&deal, &index.known()));
    assert!(!why.is_empty() && why.len() <= 3, "{why:?}");
    let said = why.join(" · ");
    assert!(
        said.contains("no-reply@shop.example")
            || said.contains("sent to many")
            || said.contains("takes no answer"),
        "{said}"
    );
    // Its own words, not a token's.
    assert!(!said.contains("from:") && !said.contains("w:"), "{said}");
}

#[test]
fn the_debrief_says_what_was_thrown_away_on_its_own_and_learnt_since() {
    let dir = tempfile::tempdir().unwrap();
    let (mut index, deal) = taught_index(dir.path(), &identity());
    let before = index.debrief(i64::MAX);
    assert!(before.thrown.is_empty() && before.decided == 0);
    index.throw_on_its_own(&deal.key).unwrap();
    let d = index.debrief(0);
    assert_eq!(d.thrown.len(), 1);
    let t = &d.thrown[0];
    assert_eq!(t.subject, "Last chance on the offer");
    assert_eq!(t.message_id, deal.message_id);
    assert!(!t.why.is_empty());
    // Your sixty decisions; not the one it made.
    assert_eq!(d.decided, 60);
    assert_eq!((d.kept, d.thrown_by_you), (30, 30));
    // Since after it: nothing.
    assert!(index.debrief(t.at + 1).thrown.is_empty());
}

/// The demo's map, taught that the CI's mail is thrown away and the rest
/// kept, as many times as it takes to be sure.
pub(super) fn demo_taught() -> App {
    let mut app = demo_map();
    {
        let mut index = app.index.lock().unwrap();
        let (ci, rest): (Vec<_>, Vec<_>) = index
            .records
            .values()
            .map(|r| (r.key.clone(), r.from == "CI"))
            .partition(|(_, ci)| *ci);
        // One of the CI's left untaught, for it to be sure of alone.
        assert!(ci.len() >= 2 && rest.len() >= 25);
        for k in 0..40 {
            index.teach(&ci[1 + k % (ci.len() - 1)].0, true).unwrap();
            index.teach(&rest[k % 25].0, false).unwrap();
        }
    }
    app.settle(Duration::from_secs(5));
    app
}

#[test]
fn new_mail_it_is_sure_of_is_asked_about_never_thrown_away_unasked() {
    let mut app = demo_taught();
    let auto = |app: &App| {
        app.index
            .lock()
            .unwrap()
            .filter
            .taught
            .iter()
            .filter(|t| t.auto)
            .count()
    };
    app.auto = crate::filter::Auto {
        on: true,
        since: 0,
        debriefed: 0,
    };
    app.throw_away_on_its_own();
    app.settle(Duration::from_secs(5));
    // Said, never a pop-up of its own: a key pressed as it comes must not
    // act on it. Nothing moved.
    assert!(!app.dialog.is_open(), "no pop-up unasked");
    assert!(app.status.contains("look like junk"), "{}", app.status);
    // Said at the foot while there is any, to be seen whenever looked at.
    app.status.clear();
    let screen = app::screen(&mut app, 140, 40);
    assert!(screen.contains("look like junk · J"), "{screen}");
    press(&mut app, &["J"]);
    let app::Dialog::Junk(pick) = &app.dialog else {
        panic!("J shows them: {}", app.status);
    };
    let held = &pick.items;
    assert!(!held.is_empty());
    assert!(held.iter().all(|h| h.from == "CI"), "{held:?}");
    assert!(
        held.iter().all(|h| !h.chosen),
        "none chosen till you choose"
    );
    let subject = held[0].subject.clone();
    assert_eq!(auto(&app), 0);
    assert!(dot_labels(&app, &subject).1, "{subject} still in the Inbox");
    let screen = app::screen(&mut app, 140, 40);
    assert!(screen.contains("look like junk"), "{screen}");
    assert!(screen.contains(&subject), "{screen}");
    // Any other key leaves them; said once, not again.
    press(&mut app, &["x"]);
    assert!(!app.dialog.is_open());
    assert!(dot_labels(&app, &subject).1);
    app.status.clear();
    app.throw_away_on_its_own();
    assert!(!app.status.contains("look like junk"), "said once");
    assert_eq!(auto(&app), 0);
}

#[test]
fn i_in_the_junk_pop_up_looks_through_the_inbox() {
    let mut app = demo_taught();
    app.auto = crate::filter::Auto {
        on: true,
        since: 0,
        debriefed: 0,
    };
    app.throw_away_on_its_own();
    press(&mut app, &["J"]);
    assert!(matches!(app.dialog, app::Dialog::Junk(_)));
    press(&mut app, &["i"]);
    assert!(app.review.is_some(), "{}", app.status);
    // Each suggested, none chosen.
    assert!(app.map.region.is_none());
}

#[test]
fn d_in_the_junk_pop_up_throws_those_listed_away_never_learnt() {
    let mut app = demo_taught();
    let auto = |app: &App| -> Vec<crate::filter::Taught> {
        let index = app.index.lock().unwrap();
        index
            .filter
            .taught
            .iter()
            .filter(|t| t.auto)
            .cloned()
            .collect()
    };
    let thrown = app.index.lock().unwrap().filter.thrown;
    // Off, or on only for mail come since: nothing.
    app.throw_away_on_its_own();
    app.auto = crate::filter::Auto {
        on: true,
        since: i64::MAX,
        debriefed: 0,
    };
    app.throw_away_on_its_own();
    assert!(auto(&app).is_empty());
    // On for all of it: the CI's mail it is sure of, asked about; d, as
    // chosen, out of the Inbox.
    app.auto.since = 0;
    app.throw_away_on_its_own();
    choose_all_junk(&mut app);
    press(&mut app, &["d"]);
    app.settle(Duration::from_secs(5));
    let gone = auto(&app);
    assert!(!gone.is_empty());
    assert!(
        gone.iter()
            .all(|t| t.from == "CI" && t.odds >= crate::filter::AUTO),
        "{gone:?}"
    );
    // Out of the Inbox: in the Trash, which the map does not show.
    let s = app.map.snapshot.clone().unwrap();
    for t in &gone {
        assert!(
            s.dots
                .iter()
                .filter(|d| d.subject == t.subject)
                .all(|d| !d.system.inbox),
            "{} out of the Inbox",
            t.subject
        );
    }
    assert!(app.status.contains("B says which"), "{}", app.status);
    // Not learnt as thrown away; and not thrown away twice.
    assert_eq!(app.index.lock().unwrap().filter.thrown, thrown);
    app.throw_away_on_its_own();
    assert_eq!(auto(&app).len(), gone.len());
}

#[test]
fn letterbox_opens_with_a_debrief_and_b_opens_it_again() {
    let mut app = demo_taught();
    app.auto = crate::filter::Auto {
        on: true,
        since: 0,
        debriefed: 0,
    };
    // Nothing thrown away on its own: nothing to tell.
    app.debrief_on_start();
    assert!(!matches!(app.dialog, app::Dialog::Debrief(_)));
    app.throw_away_on_its_own();
    choose_all_junk(&mut app);
    press(&mut app, &["d"]);
    app.settle(Duration::from_secs(5));
    app.debrief_on_start();
    let app::Dialog::Debrief(d) = &app.dialog else {
        panic!("no debrief: {}", app.status);
    };
    let n = d.thrown.len();
    let first = d.thrown[0].subject.clone();
    assert!(n > 0);
    assert!(app.auto.debriefed > 0, "told of");
    let screen = app::screen(&mut app, 120, 40);
    assert!(screen.contains("Since you last looked"), "{screen}");
    assert!(screen.contains(&first), "{screen}");
    assert!(
        screen.contains("sent to many") || screen.contains("list "),
        "{screen}"
    );
    // ⏎ puts the one picked back: learnt as kept.
    let kept = app.index.lock().unwrap().filter.kept;
    press(&mut app, &["<enter>"]);
    app.settle(Duration::from_secs(5));
    assert_eq!(app.index.lock().unwrap().filter.kept, kept + 1);
    assert!(dot_labels(&app, &first).1, "{first} back in the Inbox");
    let screen = app::screen(&mut app, 120, 40);
    assert!(screen.contains("put back"), "{screen}");
    // esc: told of, so not again as it starts.
    press(&mut app, &["<esc>"]);
    assert!(!app.dialog.is_open());
    app.debrief_on_start();
    assert!(!app.dialog.is_open());
    // B tells of the last seven days again; u puts the rest back.
    press(&mut app, &["B"]);
    let app::Dialog::Debrief(d) = &app.dialog else {
        panic!("B: {}", app.status);
    };
    assert_eq!(d.thrown.len(), n);
    assert_eq!(d.thrown.iter().filter(|t| t.back).count(), 1);
    press(&mut app, &["u"]);
    app.settle(Duration::from_secs(5));
    assert_eq!(app.index.lock().unwrap().filter.kept, kept + n as u32);
}

#[test]
fn b_with_nothing_thrown_away_says_so() {
    let mut app = demo();
    press(&mut app, &["B"]);
    assert!(!app.dialog.is_open());
    assert!(
        app.status.contains("nothing thrown away as junk"),
        "{}",
        app.status
    );
}

#[test]
fn whether_it_throws_away_on_its_own_is_kept_beside_the_accounts() {
    // None yet: on, for mail come from now.
    let a = crate::filter::Auto::read("", 100);
    assert_eq!((a.on, a.since, a.debriefed), (true, 100, 100));
    let b = crate::filter::Auto {
        on: false,
        since: 5,
        debriefed: 9,
    };
    let again = crate::filter::Auto::read(&b.to_string(), 100);
    assert_eq!((again.on, again.since, again.debriefed), (false, 5, 9));
    // letterbox filter auto off, and on again from now.
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("filter");
    crate::set_auto(&path, false, 50).unwrap();
    let off = crate::filter::Auto::read(&std::fs::read_to_string(&path).unwrap(), 0);
    assert!(!off.on);
    crate::set_auto(&path, true, 70).unwrap();
    let on = crate::filter::Auto::read(&std::fs::read_to_string(&path).unwrap(), 0);
    assert!(on.on && on.since == 70);
}

#[test]
fn filter_debrief_says_it_on_the_command_line() {
    let dir = tempfile::tempdir().unwrap();
    let (mut index, deal) = taught_index(dir.path(), &identity());
    index.throw_on_its_own(&deal.key).unwrap();
    let said = crate::debrief_text(&index, 0);
    assert!(said.contains("thrown away on its own: 1"), "{said}");
    assert!(said.contains("Last chance on the offer"), "{said}");
    assert!(said.contains("60 of your decisions"), "{said}");
}

/// J, and each listed chosen, by space, one after another.
fn choose_all_junk(app: &mut App) {
    press(app, &["J"]);
    let n = match &app.dialog {
        app::Dialog::Junk(pick) => pick.items.len(),
        _ => panic!("J shows them: {}", app.status),
    };
    for _ in 0..n {
        press(app, &[" "]);
    }
}

#[test]
fn d_in_the_junk_list_with_nothing_chosen_throws_nothing() {
    let mut app = demo_taught();
    app.auto = crate::filter::Auto {
        on: true,
        since: 0,
        debriefed: 0,
    };
    app.throw_away_on_its_own();
    let held = app.junk_held();
    assert!(held > 0);
    press(&mut app, &["J", "d"]);
    assert!(matches!(app.dialog, app::Dialog::Junk(_)), "still open");
    assert!(app.status.contains("nothing chosen"), "{}", app.status);
    assert!(
        app.index
            .lock()
            .unwrap()
            .filter
            .taught
            .iter()
            .all(|t| !t.auto)
    );
    // One chosen: that one alone.
    press(&mut app, &[" ", "d"]);
    app.settle(Duration::from_secs(5));
    let n = app
        .index
        .lock()
        .unwrap()
        .filter
        .taught
        .iter()
        .filter(|t| t.auto)
        .count();
    assert_eq!(n, 1, "{}", app.status);
    // Held no more; the rest still, for J again.
    assert_eq!(app.junk_held(), held - 1);
}

#[test]
fn the_junk_list_says_why_and_n_says_one_is_not_junk_learnt_as_kept() {
    let mut app = demo_taught();
    app.auto = crate::filter::Auto {
        on: true,
        since: 0,
        debriefed: 0,
    };
    app.throw_away_on_its_own();
    let held = app.junk_held();
    press(&mut app, &["J"]);
    let (why, key) = match &app.dialog {
        app::Dialog::Junk(pick) => (pick.items[0].why.clone(), pick.items[0].key.clone()),
        _ => panic!("J shows them: {}", app.status),
    };
    // Why it is thought junk, the things that weighed most, to be seen.
    assert!(!why.is_empty());
    let screen = app::screen(&mut app, 150, 45);
    assert!(screen.contains(&why[0]), "{why:?} in {screen}");
    // n: not junk, as you say; learnt so, and asked of no more.
    let kept = app.index.lock().unwrap().filter.kept;
    press(&mut app, &["n"]);
    assert_eq!(app.index.lock().unwrap().filter.kept, kept + 1);
    assert_eq!(app.junk_held(), held - 1);
    if let app::Dialog::Junk(pick) = &app.dialog {
        assert!(pick.items.iter().all(|j| j.key != key));
    }
    assert!(
        dot_labels(
            &app,
            &app.index.lock().unwrap().records[&key].subject.clone()
        )
        .1,
        "still in the Inbox"
    );
}
