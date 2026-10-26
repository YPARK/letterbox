//! Passwords moved between the keyring and the vault.

use super::*;

// ---- Moving the passwords between the keyring and the vault ----

/// A stand-in for both stores, by where, host and user.
pub(super) static KEPT: std::sync::Mutex<
    std::collections::BTreeMap<(String, String, String), String>,
> = std::sync::Mutex::new(std::collections::BTreeMap::new());

pub(super) fn kept_at(k: Keeper, host: &str, user: &str) -> (String, String, String) {
    (k.name().to_string(), host.to_string(), user.to_string())
}

pub(super) fn fake_find(k: Keeper, host: &str, user: &str) -> anyhow::Result<String> {
    KEPT.lock()
        .unwrap()
        .get(&kept_at(k, host, user))
        .cloned()
        .ok_or_else(|| anyhow::anyhow!("none kept for {user} at {host}"))
}

pub(super) fn fake_keep(k: Keeper, host: &str, user: &str, secret: &str) -> anyhow::Result<()> {
    KEPT.lock()
        .unwrap()
        .insert(kept_at(k, host, user), secret.to_string());
    Ok(())
}

pub(super) fn fake_forget(k: Keeper, host: &str, user: &str) -> anyhow::Result<()> {
    KEPT.lock().unwrap().remove(&kept_at(k, host, user));
    Ok(())
}

#[test]
fn every_password_and_the_maps_key_move_to_the_vault_and_back() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("accounts.toml");
    std::fs::write(
        &path,
        "# my accounts\n\
         [[account]]\n\
         name = \"Work\"\n\
         email = \"me@keep.example\"\n\
         host = \"imap.keep.example\"\n\
         # its own comment\n\n\
         [[account]]\n\
         name = \"Pass\"\n\
         email = \"p@keep.example\"\n\
         host = \"imap.keep.example\"\n\
         password_command = \"echo pw\"\n\n\
         [[account]]\n\
         name = \"Broken\"\n\
         email = \"b@keep.example\"\n\
         host = \"imap.broken.example\"\n",
    )
    .unwrap();
    let map = dir.path().join("map");
    crate::index::key::write(
        &map,
        &crate::index::key::Card {
            version: 1,
            recipient: "age1keep".to_string(),
            keeper: Keeper::Keyring,
        },
    )
    .unwrap();
    fake_keep(
        Keeper::Keyring,
        "imap.keep.example",
        "me@keep.example",
        "pw1",
    )
    .unwrap();
    fake_keep(Keeper::Keyring, "map", "identity", "AGE-SECRET-KEY-1").unwrap();
    // One it cannot read: nothing of it is changed, and it says so.
    let e = crate::setup::move_secrets(
        &path,
        &map,
        Keeper::Vault,
        fake_find,
        fake_keep,
        fake_forget,
    )
    .unwrap_err();
    assert!(format!("{e:#}").contains("Broken"), "{e:#}");
    let accounts = crate::config::load(&path).unwrap();
    assert_eq!(accounts[0].keep, Keeper::Vault, "Work, before it, moved");
    assert_eq!(accounts[2].keep, Keeper::Keyring);
    fake_keep(
        Keeper::Keyring,
        "imap.broken.example",
        "b@keep.example",
        "pw3",
    )
    .unwrap();
    // All of them, then: each kept in the vault, and gone from the keyring.
    let said = crate::setup::move_secrets(
        &path,
        &map,
        Keeper::Vault,
        fake_find,
        fake_keep,
        fake_forget,
    )
    .unwrap();
    assert!(
        said.iter().any(|l| l.starts_with("Broken: kept")),
        "{said:?}"
    );
    assert!(
        said.iter().any(|l| l.starts_with("the map's key: kept")),
        "{said:?}"
    );
    assert!(
        said.iter().any(|l| l.contains("password_command")),
        "{said:?}"
    );
    let get = |k, h: &str, u: &str| fake_find(k, h, u).ok();
    assert_eq!(
        get(Keeper::Vault, "imap.keep.example", "me@keep.example").as_deref(),
        Some("pw1")
    );
    assert_eq!(
        get(Keeper::Vault, "imap.broken.example", "b@keep.example").as_deref(),
        Some("pw3")
    );
    assert_eq!(
        get(Keeper::Vault, "map", "identity").as_deref(),
        Some("AGE-SECRET-KEY-1")
    );
    assert!(get(Keeper::Keyring, "imap.keep.example", "me@keep.example").is_none());
    assert!(get(Keeper::Keyring, "map", "identity").is_none());
    let accounts = crate::config::load(&path).unwrap();
    assert!(
        accounts
            .iter()
            .all(|a| a.keep == Keeper::Vault || a.name == "Pass")
    );
    assert_eq!(accounts[1].keep, Keeper::Keyring, "a command's is left");
    let card = crate::index::key::card(&map).unwrap().unwrap();
    assert_eq!(card.keeper, Keeper::Vault);
    assert_eq!(card.recipient, "age1keep");
    let text = std::fs::read_to_string(&path).unwrap();
    assert!(
        text.contains("# my accounts") && text.contains("# its own comment"),
        "{text}"
    );
    // Again: nothing to move.
    assert!(
        crate::setup::move_secrets(
            &path,
            &map,
            Keeper::Vault,
            fake_find,
            fake_keep,
            fake_forget
        )
        .unwrap()
        .iter()
        .all(|l| l.contains("password_command"))
    );
    // And back: the file says nothing of where, as the keyring is the default.
    crate::setup::move_secrets(
        &path,
        &map,
        Keeper::Keyring,
        fake_find,
        fake_keep,
        fake_forget,
    )
    .unwrap();
    let text = std::fs::read_to_string(&path).unwrap();
    assert!(!text.contains("keep ="), "{text}");
    assert_eq!(
        get(Keeper::Keyring, "map", "identity").as_deref(),
        Some("AGE-SECRET-KEY-1")
    );
    assert!(get(Keeper::Vault, "map", "identity").is_none());
}

#[test]
fn mail_that_differs_by_its_number_alone_is_one_thread() {
    use crate::forest::thread_of;
    let failed = |n: u32| {
        thread_of(&format!(
            "Retrieval using the IMAP4 protocol failed for the following message: {n}"
        ))
    };
    assert_eq!(failed(839), failed(8));
    assert_eq!(
        failed(12),
        "Retrieval using the IMAP4 protocol failed for the following message: …"
    );
    assert_eq!(thread_of("Re: Fwd: Invoice #4471"), "Invoice #…");
    assert_ne!(thread_of("Aims page"), thread_of("Aims page, third pass"));
}

#[test]
fn the_folders_that_hold_no_mail_are_not_mapped() {
    use crate::mail::Folder;
    let folders = vec![
        Folder::new("INBOX", Some("/"), None, true),
        Folder::new("Calendar", Some("/"), None, true),
        Folder::new("Calendar/Birthdays", Some("/"), None, true),
        Folder::new("Contacts", Some("/"), None, true),
        Folder::new("Sync Issues/Conflicts", Some("/"), None, true),
        Folder::new("Projects/Calendar talk", Some("/"), None, true),
        Folder::new("Notes", None, None, true),
    ];
    let mapped: Vec<&str> = crate::index::sync::mapped(&folders, crate::store::Kind::Folders)
        .iter()
        .map(|f| f.path.as_str())
        .collect();
    assert_eq!(mapped, ["INBOX", "Projects/Calendar talk"]);
}

#[test]
fn the_view_shows_only_the_mail_not_yet_filed() {
    let mut app = demo_map();
    let now = jiff::Timestamp::now().as_second();
    let all = app.map.shown(now).len();
    press(&mut app, &[",", "f", "<esc>"]);
    assert!(app.map.unfiled_only);
    let s = app.map.snapshot.clone().unwrap();
    let shown = app.map.shown(now);
    assert!(!shown.is_empty() && shown.len() < all);
    assert!(
        shown
            .iter()
            .all(|&i| s.dots[i].labels.is_empty() && !s.dots[i].system.sent)
    );
    let screen = app::screen(&mut app, 150, 45);
    assert!(screen.contains("not yet filed"), "{screen}");
    press(&mut app, &[",", "f", "<esc>"]);
    assert_eq!(app.map.shown(now).len(), all);
}

#[test]
fn mail_not_yet_filed_is_said_where_it_would_go_and_filed_there() {
    let mut app = demo_map();
    press(&mut app, &[",", "f", "<esc>"]);
    let now = jiff::Timestamp::now().as_second();
    let s = app.map.snapshot.clone().unwrap();
    let going: Vec<(usize, String)> = app
        .map
        .shown(now)
        .into_iter()
        .filter_map(|i| Some((i, s.suggestion(i)?.to_string())))
        .collect();
    assert!(!going.is_empty(), "some would go to a label");
    let (first, label) = going[0].clone();
    // Where it would go, a label there is, as the filed mail nearest it
    // votes.
    assert!(
        s.model
            .label_vectors
            .contains_key(&crate::index::fold(&label)),
        "{label}"
    );
    app.map.cursor = Some(s.dots[first].key.clone());
    read_on_map(&mut app);
    let screen = app::screen(&mut app, 150, 60);
    assert!(screen.contains(&format!("Would go to {label}")), "{screen}");
    // y: filed there.
    press(&mut app, &["y"]);
    let subject = &s.dots[first].subject;
    let (labels, _) = dot_labels(&app, subject);
    assert!(labels.contains(&label), "{labels:?}");
    // -: not it, and it is suggested there no more.
    let s = app.map.snapshot.clone().unwrap();
    let would = |app: &App, s: &crate::map::Snapshot| -> Vec<usize> {
        app.map
            .shown(now)
            .into_iter()
            .filter(|&i| s.suggestion(i) == Some(label.as_str()))
            .collect()
    };
    let turned = would(&app, &s)[0];
    app.map.cursor = Some(s.dots[turned].key.clone());
    press(&mut app, &["-"]);
    let s = app.map.snapshot.clone().unwrap();
    let at = s.find(&s.dots[turned].key.clone()).unwrap();
    assert_ne!(s.suggestion(at), Some(label.as_str()));
    // Y: the rest that would go there marked, and, again, filed.
    let rest = would(&app, &s);
    assert!(rest.len() >= 2, "{}", rest.len());
    app.map.cursor = Some(s.dots[rest[0]].key.clone());
    press(&mut app, &["Y"]);
    assert_eq!(app.map.region.as_ref().unwrap().keys.len(), rest.len());
    assert!(app.status.contains("marked"), "{}", app.status);
    press(&mut app, &["Y"]);
    for &i in &rest {
        let (labels, _) = dot_labels(&app, &s.dots[i].subject);
        assert!(labels.contains(&label), "{}: {labels:?}", s.dots[i].subject);
    }
}

#[test]
fn the_message_written_is_sent_or_discarded_by_buttons_on_its_top_edge() {
    let mut app = demo();
    press(&mut app, &["<enter>", "r"]);
    assert!(app.compose.is_some());
    let screen = app::screen(&mut app, 160, 40);
    let top = screen
        .lines()
        .find(|l| l.contains("From "))
        .unwrap()
        .to_string();
    for b in ["Send  C-c C-c", "Attach  C-c C-a", "Discard  C-c C-k"] {
        assert!(top.contains(b), "{top}");
    }
    let discard = (
        crate::keymap::key_named("C-c").unwrap(),
        crate::keymap::key_named("C-k").unwrap(),
    );
    let (rect, _) = *app
        .targets
        .iter()
        .filter(|(_, t)| *t == app::Target::Chord(discard.0, discard.1))
        .min_by_key(|(r, _)| r.y)
        .unwrap();
    app.mouse(mouse(
        MouseEventKind::Down(MouseButton::Left),
        rect.x + 1,
        rect.y,
    ));
    assert!(app.compose.is_none(), "{}", app.status);
}

#[test]
fn the_inbox_is_looked_through_beside_the_map_and_done_at_once() {
    let mut app = demo_map();
    press(&mut app, &["i"]);
    assert!(app.review.is_some());
    assert_eq!(app.focus, app::Focus::Reader);
    let rows = app.review_rows();
    assert_eq!(rows.len(), INBOX);
    let screen = app::screen(&mut app, 170, 40);
    assert!(screen.contains("Inbox · 33 to look through"), "{screen}");
    assert!(screen.contains("Apply  $"), "{screen}");
    // Nothing chosen until it is: what is suggested only hinted at, and $
    // has nothing to do.
    assert!(!screen.contains('✓'), "{screen}");
    assert!(screen.contains("[?] "), "{screen}");
    assert!(screen.contains("none chosen"), "{screen}");
    assert!(rows.iter().all(|&i| app.chosen(i).is_none()));
    press(&mut app, &["$"]);
    assert!(app.status.contains("nothing chosen"), "{}", app.status);
    assert!(!app.dialog.is_open());
    let s = app.map.snapshot.clone().unwrap();
    // The one picked is picked on the map too.
    assert_eq!(app.map.picked(), Some(rows[0]));
    // d discards it, and goes on; space keeps; y takes what is suggested.
    press(&mut app, &["d"]);
    assert_eq!(app.choice(rows[0]), app::Choice::Discard);
    assert_eq!(app.map.picked(), Some(rows[1]));
    press(&mut app, &["k", "<space>"]);
    assert_eq!(app.choice(rows[0]), app::Choice::Keep);
    // ← and → go round what may be done, and round to none chosen.
    press(&mut app, &["k", "<right>"]);
    assert_eq!(app.choice(rows[0]), app::Choice::Discard);
    press(&mut app, &["<right>"]);
    assert_eq!(app.chosen(rows[0]), None);
    press(&mut app, &["<left>"]);
    assert_eq!(app.chosen(rows[0]), Some(app::Choice::Discard));
    // One suggested to go somewhere: y files it there.
    let going = rows
        .iter()
        .position(|&i| s.suggestion(i).is_some())
        .unwrap();
    let label = s.suggestion(rows[going]).unwrap().to_string();
    for _ in 0..going {
        press(&mut app, &["j"]);
    }
    press(&mut app, &["d", "k", "y"]);
    assert_eq!(app.choice(rows[going]), app::Choice::Move(label.clone()));
    // One with nowhere to go: m picks where on the map; esc comes back.
    let nowhere = rows
        .iter()
        .position(|&i| s.suggestion(i).is_none() && i != rows[0])
        .unwrap();
    app.focus = app::Focus::Reader;
    let key = s.dots[rows[nowhere]].key.clone();
    app.review.as_mut().unwrap().picked = Some(key.clone());
    press(&mut app, &["m", "<up>", "<enter>"]);
    assert_eq!(app.focus, app::Focus::List);
    assert!(app.map.moving.is_some());
    press(&mut app, &["<esc>"]);
    assert_eq!(app.focus, app::Focus::Reader);
    assert!(app.review.as_ref().unwrap().relabel.is_none());
    // Again, and a click on a label: chosen, not done yet.
    app.review.as_mut().unwrap().picked = Some(key.clone());
    press(&mut app, &["m", "<up>", "<enter>"]);
    let grant = s
        .groups
        .iter()
        .find(|g| g.label.as_deref() == Some("Grant"))
        .unwrap()
        .id;
    press(&mut app, &["0"]);
    click_at(&mut app, app::Target::Anchor(grant));
    assert_eq!(app.choice(rows[nowhere]), app::Choice::Move("Grant".into()));
    assert!(
        dot_labels(&app, &s.dots[rows[nowhere]].subject).1,
        "still in the Inbox"
    );
    // $ asks, saying how many are left; n leaves it; $ and y do those
    // chosen, and only those.
    app.focus = app::Focus::Reader;
    press(&mut app, &["$"]);
    let screen = app::screen(&mut app, 170, 40);
    assert!(screen.contains("left as they are"), "{screen}");
    press(&mut app, &["n"]);
    assert!(app.review.as_ref().unwrap().chosen.len() >= 2);
    let untouched: Vec<String> = rows
        .iter()
        .filter(|&&i| app.chosen(i).is_none())
        .map(|&i| s.dots[i].subject.clone())
        .collect();
    assert!(!untouched.is_empty());
    press(&mut app, &["$", "y"]);
    for subject in &untouched {
        assert!(dot_labels(&app, subject).1, "{subject} left in the Inbox");
    }
    let (labels, inbox) = dot_labels(&app, &s.dots[rows[going]].subject);
    assert!(labels.contains(&label) && !inbox, "{labels:?}");
    let (labels, inbox) = dot_labels(&app, &s.dots[rows[nowhere]].subject);
    assert!(
        labels.contains(&"Grant".to_string()) && !inbox,
        "{labels:?}"
    );
    assert!(app.status.contains("filed"), "{}", app.status);
    let trashed = &s.dots[rows[0]].subject;
    let s = app.map.snapshot.clone().unwrap();
    assert!(
        s.dots
            .iter()
            .filter(|d| &d.subject == trashed)
            .all(|d| !d.system.inbox),
        "discarded"
    );
    // How it went, said; any key closes that, and i the rest.
    assert!(matches!(app.dialog, app::Dialog::Applied(_)));
    press(&mut app, &["x", "i"]);
    assert!(app.review.is_none());
}

#[test]
fn what_is_chosen_in_the_inbox_is_asked_in_a_pop_up_before_it_is_done() {
    let mut app = demo_map();
    press(&mut app, &["i"]);
    let rows = app.review_rows();
    // The first discarded, the second kept.
    press(&mut app, &["d", "<space>"]);
    press(&mut app, &["$"]);
    assert!(matches!(app.dialog, app::Dialog::Apply(_)));
    let screen = app::screen(&mut app, 170, 40);
    assert!(screen.contains("Do what was chosen?"), "{screen}");
    assert!(screen.contains("Discard 1 to the Trash"), "{screen}");
    assert!(screen.contains("keep 1 in the Inbox"), "{screen}");
    let left = INBOX - 2;
    assert!(screen.contains(&format!("The other {left}")), "{screen}");
    assert!(
        screen.contains("Do it") && screen.contains("Not yet"),
        "{screen}"
    );
    // n: nothing done, what was chosen still chosen.
    press(&mut app, &["n"]);
    assert!(!app.dialog.is_open());
    assert_eq!(app.chosen(rows[0]), Some(app::Choice::Discard));
    assert!(app.status.contains("nothing done"), "{}", app.status);
    // Not yet, clicked: the same.
    press(&mut app, &["$"]);
    click_at(&mut app, app::Target::Button(app::Button::NotYet));
    assert!(!app.dialog.is_open());
    assert_eq!(app.chosen(rows[1]), Some(app::Choice::Keep));
    // Do it, clicked: done.
    press(&mut app, &["$"]);
    click_at(&mut app, app::Target::Button(app::Button::Apply));
    assert!(!app.dialog.is_open());
    assert!(app.review.as_ref().unwrap().chosen.is_empty());
    assert!(app.status.contains("1 kept"), "{}", app.status);
}

#[test]
fn only_what_the_pop_up_said_is_done() {
    let mut app = demo_map();
    press(&mut app, &["i"]);
    let rows = app.review_rows();
    let s = app.map.snapshot.clone().unwrap();
    press(&mut app, &["<space>", "$"]);
    // Chosen after it was asked, as a pass of the map might change what is
    // there: not done, as it was not said.
    let later = s.dots[rows[1]].key.clone();
    app.review
        .as_mut()
        .unwrap()
        .chosen
        .insert(later, app::Choice::Discard);
    press(&mut app, &["y"]);
    assert!(app.status.contains("1 kept"), "{}", app.status);
    assert!(!app.status.contains("Trash"), "{}", app.status);
    assert!(
        dot_labels(&app, &s.dots[rows[1]].subject).1,
        "still in the Inbox"
    );
}

#[test]
fn the_pop_up_names_the_labels_filed_under() {
    let mut app = demo_map();
    press(&mut app, &["i"]);
    let s = app.map.snapshot.clone().unwrap();
    let rows = app.review_rows();
    let going = rows
        .iter()
        .position(|&i| s.suggestion(i).is_some())
        .unwrap();
    let label = s.suggestion(rows[going]).unwrap().to_string();
    for _ in 0..going {
        press(&mut app, &["j"]);
    }
    press(&mut app, &["y", "$"]);
    let screen = app::screen(&mut app, 170, 40);
    assert!(
        screen.contains(&format!("File 1 under {label}")),
        "{screen}"
    );
    press(&mut app, &["y"]);
    assert!(!app.dialog.is_open());
    assert!(!dot_labels(&app, &s.dots[rows[going]].subject).1);
}

#[test]
fn as_the_inbox_is_looked_through_the_map_says_where_it_is_heading() {
    let mut app = demo_map();
    // Not looked through: nothing said of it.
    assert!(app.heading().is_none());
    let screen = app::screen(&mut app, 170, 40);
    assert!(!screen.contains("where the Inbox goes"), "{screen}");
    press(&mut app, &["i"]);
    let s = app.map.snapshot.clone().unwrap();
    let rows = app.review_rows();
    // Each where it would go, as suggested: to a label, to the Trash, or
    // kept; all of them, somewhere.
    let h = app.heading().expect("heading");
    let to: usize = h.to.values().map(|w| w.chosen + w.suggested).sum();
    let unmapped = rows
        .iter()
        .filter(|&&i| matches!(app.choice(i), app::Choice::Move(ref l) if !s.groups.iter().any(|g| g.label.as_deref() == Some(l))))
        .count();
    assert_eq!(
        to + h.trash.chosen + h.trash.suggested + h.kept.chosen + h.kept.suggested + unmapped,
        INBOX
    );
    assert!(h.to.values().all(|w| w.chosen == 0), "none chosen yet");
    // The one picked, to where it is suggested.
    let going = rows
        .iter()
        .position(|&i| s.suggestion(i).is_some())
        .unwrap();
    for _ in 0..going {
        press(&mut app, &["j"]);
    }
    let label = s.suggestion(rows[going]).unwrap().to_string();
    let there = s
        .groups
        .iter()
        .find(|g| g.label.as_deref() == Some(label.as_str()))
        .unwrap()
        .id;
    let h = app.heading().unwrap();
    assert_eq!(h.picked, Some(app::Bound::To(there)));
    let before = h.to[&there];
    // Revised: to the Trash; there one fewer, and the Trash one more, chosen.
    press(&mut app, &["d", "k"]);
    let after = app.heading().unwrap();
    assert_eq!(after.picked, Some(app::Bound::Trash));
    assert_eq!(
        after.to.get(&there).map_or(0, |w| w.suggested),
        before.suggested - 1
    );
    assert_eq!(after.trash.chosen, 1);
    // Revised again, to Grant: chosen there.
    let grant = s
        .groups
        .iter()
        .find(|g| g.label.as_deref() == Some("Grant"))
        .unwrap()
        .id;
    let was = app
        .heading()
        .unwrap()
        .to
        .get(&grant)
        .map_or(0, |w| w.chosen);
    app.review.as_mut().unwrap().chosen.insert(
        s.dots[rows[going]].key.clone(),
        app::Choice::Move("Grant".into()),
    );
    let h = app.heading().unwrap();
    assert_eq!(h.to[&grant].chosen, was + 1);
    assert_eq!(h.picked, Some(app::Bound::To(grant)));
    // Drawn: said under the map, and how many go to each label beside it.
    let screen = app::screen(&mut app, 170, 40);
    assert!(screen.contains("where the Inbox goes"), "{screen}");
    let going = h.to[&grant].all();
    assert!(
        screen.contains(&format!("← {going} of the Inbox")),
        "{screen}"
    );
}

#[test]
fn a_message_looked_through_goes_under_a_new_label_named_for_it() {
    let mut app = demo_map();
    press(&mut app, &["i"]);
    let rows = app.review_rows();
    let s = app.map.snapshot.clone().unwrap();
    let subject = s.dots[rows[0]].subject.clone();
    // N asks its name, its commonest word given; esc lets it go.
    press(&mut app, &["N"]);
    assert!(matches!(app.dialog, app::Dialog::Label(_)));
    press(&mut app, &["<esc>"]);
    assert_eq!(app.focus, app::Focus::Reader);
    assert!(app.map.moving.is_none());
    assert_eq!(app.choice(rows[0]), app::App::suggested(&s, rows[0]));
    // Named: chosen, not yet done; the new label beside it.
    press(&mut app, &["N", "<c-u>"]);
    for c in "Engines".chars() {
        let shift = if c.is_uppercase() {
            KeyModifiers::SHIFT
        } else {
            KeyModifiers::NONE
        };
        app.key(KeyEvent::new(KeyCode::Char(c), shift));
    }
    press(&mut app, &["<enter>"]);
    assert_eq!(app.choice(rows[0]), app::Choice::Move("Engines".into()));
    assert!(dot_labels(&app, &subject).1, "still in the Inbox");
    let screen = app::screen(&mut app, 170, 40);
    assert!(screen.contains("→ Engines"), "{screen}");
    // $ and y: made, and filed under it.
    app.focus = app::Focus::Reader;
    press(&mut app, &["$", "y"]);
    let (labels, inbox) = dot_labels(&app, &subject);
    assert!(
        labels.contains(&"Engines".to_string()) && !inbox,
        "{labels:?}"
    );
}

#[test]
fn the_labels_drop_down_nearest_first_narrowed_as_typed() {
    let mut app = demo_map();
    press(&mut app, &["i"]);
    let rows = app.review_rows();
    let s = app.map.snapshot.clone().unwrap();
    // One that would go to Grant: Grant first, the nearest few after it.
    let going = rows
        .iter()
        .position(|&i| s.suggestion(i) == Some("Grant"))
        .unwrap();
    let i = rows[going];
    app.review.as_mut().unwrap().picked = Some(s.dots[i].key.clone());
    press(&mut app, &["m"]);
    assert!(app.menu.is_some());
    let items = app.menu_items();
    assert!(
        matches!(&items[0], app::Pick::Label(n, _) if n == "Grant"),
        "{items:?}"
    );
    assert!(items.len() <= 5 + 1);
    assert_eq!(items.last(), Some(&app::Pick::OnMap));
    let screen = app::screen(&mut app, 170, 40);
    assert!(
        screen.contains("Move to") && screen.contains("pick on the map"),
        "{screen}"
    );
    // Typed, narrowed; a name no label has, a new one; ⏎ moves it there.
    for c in "proj".chars() {
        app.key(KeyEvent::new(KeyCode::Char(c), KeyModifiers::NONE));
    }
    let items = app.menu_items();
    assert!(
        matches!(&items[0], app::Pick::Label(n, _) if n == "Projects"),
        "{items:?}"
    );
    assert!(
        !items
            .iter()
            .any(|p| matches!(p, app::Pick::Label(n, _) if n == "Grant"))
    );
    press(&mut app, &["<enter>"]);
    assert_eq!(app.choice(i), app::Choice::Move("Projects".into()));
    assert!(app.menu.is_none());
    // A new one, by its name typed.
    app.review.as_mut().unwrap().picked = Some(s.dots[i].key.clone());
    press(&mut app, &["m"]);
    for c in "Engines".chars() {
        let shift = if c.is_uppercase() {
            KeyModifiers::SHIFT
        } else {
            KeyModifiers::NONE
        };
        app.key(KeyEvent::new(KeyCode::Char(c), shift));
    }
    let items = app.menu_items();
    assert_eq!(items[0], app::Pick::New("Engines".into()), "{items:?}");
    press(&mut app, &["<enter>"]);
    assert_eq!(app.choice(i), app::Choice::Move("Engines".into()));
    // esc puts it away, nothing chosen; a click picks one.
    app.review.as_mut().unwrap().picked = Some(s.dots[i].key.clone());
    press(&mut app, &["m", "<esc>"]);
    assert!(app.menu.is_none());
    assert_eq!(app.choice(i), app::Choice::Move("Engines".into()));
    press(&mut app, &["m"]);
    click_at(&mut app, app::Target::MenuItem(0));
    assert_eq!(app.choice(i), app::Choice::Move("Grant".into()));
    // Picking on the map, from it.
    app.review.as_mut().unwrap().picked = Some(s.dots[i].key.clone());
    app.focus = app::Focus::Reader;
    press(&mut app, &["m", "<up>", "<enter>"]);
    assert!(app.map.moving.is_some());
    assert_eq!(app.focus, app::Focus::List);
}

#[test]
fn the_keys_of_the_inbox_looked_through_lose_nothing_and_act_on_the_one_picked() {
    let mut app = demo_map();
    press(&mut app, &["i", "d"]);
    let rows = app.review_rows();
    assert_eq!(app.choice(rows[0]), app::Choice::Discard);
    // esc goes back to the map, all of it kept; tab comes back.
    press(&mut app, &["<esc>"]);
    assert_eq!(app.focus, app::Focus::List);
    assert!(app.review.is_some());
    press(&mut app, &["<tab>"]);
    assert_eq!(app.focus, app::Focus::Reader);
    // i closes it, and opens it as it was.
    press(&mut app, &["i"]);
    assert!(app.review.is_none());
    press(&mut app, &["i"]);
    assert_eq!(app.choice(rows[0]), app::Choice::Discard);
    // Read, d chooses, as beside the rest; nothing is moved yet.
    press(&mut app, &["j", "<enter>"]);
    assert!(app.review.as_ref().unwrap().reading);
    press(&mut app, &["d"]);
    assert!(!app.review.as_ref().unwrap().reading);
    assert_eq!(app.choice(rows[1]), app::Choice::Discard);
    let s = app.map.snapshot.clone().unwrap();
    assert!(
        dot_labels(&app, &s.dots[rows[1]].subject).1,
        "still in the Inbox"
    );
    // u marks the one picked, not the one read before.
    let picked = app.map.picked().unwrap();
    let seen = s.dots[picked].flags.seen;
    press(&mut app, &["u"]);
    let s = app.map.snapshot.clone().unwrap();
    assert_ne!(s.dots[picked].flags.seen, seen);
}

#[test]
fn the_inbox_looked_through_takes_emacs_keys() {
    use ratatui::crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
    let mut app = demo_map();
    press(&mut app, &["i"]);
    let rows = app.review_rows();
    let at = |app: &App| {
        app.map
            .snapshot
            .as_ref()
            .unwrap()
            .find(app.review.as_ref().unwrap().picked.as_ref().unwrap())
    };
    assert_eq!(at(&app), Some(rows[0]));
    // C-n and C-p, down and up; new mail is not begun.
    press(&mut app, &["<c-n>", "<c-n>"]);
    assert_eq!(at(&app), Some(rows[2]));
    assert!(app.compose.is_none());
    press(&mut app, &["<c-p>"]);
    assert_eq!(at(&app), Some(rows[1]));
    // M-> the last, M-< the first; C-v a page down, M-v back up.
    let meta = |c| KeyEvent::new(KeyCode::Char(c), KeyModifiers::ALT);
    app.key(meta('>'));
    assert_eq!(at(&app), rows.last().copied());
    app.key(meta('<'));
    assert_eq!(at(&app), Some(rows[0]));
    press(&mut app, &["<c-v>"]);
    let paged = rows.iter().position(|&r| Some(r) == at(&app)).unwrap();
    assert!(paged > 1, "{paged}");
    app.key(meta('v'));
    assert_eq!(at(&app), Some(rows[0]));
    // C-f and C-b round what may be done with it, as → and ←; nothing is
    // forwarded.
    press(&mut app, &["<c-f>"]);
    assert_ne!(app.chosen(rows[0]), None);
    assert!(app.compose.is_none());
    press(&mut app, &["<c-b>"]);
    assert_eq!(app.chosen(rows[0]), None);
    // C-g, as esc: reading one, back to the rest; then back to the map,
    // all of it kept.
    press(&mut app, &["<enter>"]);
    assert!(app.review.as_ref().unwrap().reading);
    press(&mut app, &["<c-g>"]);
    assert!(!app.review.as_ref().unwrap().reading);
    assert_eq!(app.focus, app::Focus::Reader);
    press(&mut app, &["<c-g>"]);
    assert_eq!(app.focus, app::Focus::List);
    assert!(app.review.is_some());
}

#[test]
fn a_message_read_on_the_map_is_the_one_answered_forwarded_and_archived() {
    let mut app = demo_map();
    let listed = app.current().unwrap().summary.subject.clone();
    press(&mut app, &["i"]);
    let rows = app.review_rows();
    let s = app.map.snapshot.clone().unwrap();
    // One of the Inbox looked through, read: not the list's, behind the map.
    press(&mut app, &["j", "j", "<enter>"]);
    assert!(app.review.as_ref().unwrap().reading);
    let subject = s.dots[rows[2]].subject.clone();
    assert_ne!(subject, listed);
    let said = |app: &App| app.compose.as_ref().unwrap().buffer.text();
    press(&mut app, &["r"]);
    let reply = format!("Subject: {}", crate::compose::reply_subject(&subject));
    assert!(said(&app).contains(&reply), "{}", said(&app));
    press(&mut app, &["<c-c>", "<c-k>"]);
    press(&mut app, &["f"]);
    let forward = format!("Subject: {}", crate::compose::forward_subject(&subject));
    assert!(said(&app).contains(&forward), "{}", said(&app));
    press(&mut app, &["<c-c>", "<c-k>"]);
    // e, reading it, does nothing yet, as beside the rest: neither it nor
    // the list's is taken out of the Inbox.
    press(&mut app, &["e"]);
    assert!(dot_labels(&app, &subject).1, "{subject} still in the Inbox");
    assert!(dot_labels(&app, &listed).1, "{listed} still in the Inbox");
    // The triage closed, a cluster gone into down to a message, read:
    // that one answered, too.
    press(&mut app, &["i", "<esc>"]);
    read_on_map(&mut app);
    let p = app.map.picked().unwrap();
    let other = app.map.snapshot.as_ref().unwrap().dots[p].subject.clone();
    press(&mut app, &["r"]);
    let reply = format!("Subject: {}", crate::compose::reply_subject(&other));
    assert!(said(&app).contains(&reply), "{}", said(&app));
}
