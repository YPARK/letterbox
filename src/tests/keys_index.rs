//! The key table, the colours, and the map's index: its journals, its
//! key and its sync.

use super::*;

// ---- the key table ----

#[test]
fn every_key_in_the_table_is_read_and_bound_once() {
    use crate::keymap::{KEYS, key_named};
    use ratatui::crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
    let mut seen = std::collections::HashSet::new();
    for (name, act) in KEYS.iter().flat_map(|b| b.keys) {
        let key = key_named(name).unwrap_or_else(|| panic!("{name} is no key"));
        assert_eq!(crate::keymap::act(key), Some(*act), "{name}");
        assert!(
            seen.insert(crate::keymap::chord(key)),
            "{name} is bound twice"
        );
    }
    // Ctrl and Shift, as terminals say it or not, is no key of ours, as
    // kitty keeps ctrl+shift for itself: not C-r's.
    let ctrl_shift = KeyModifiers::CONTROL | KeyModifiers::SHIFT;
    for key in [
        KeyEvent::new(KeyCode::Char('r'), ctrl_shift),
        KeyEvent::new(KeyCode::Char('R'), ctrl_shift),
        KeyEvent::new(KeyCode::Char('R'), KeyModifiers::CONTROL),
    ] {
        assert_eq!(crate::keymap::act(key), None);
    }
    assert_eq!(
        crate::keymap::act(KeyEvent::new(KeyCode::Char('G'), KeyModifiers::SHIFT)),
        crate::keymap::act(key_named("end").unwrap()),
    );
    assert_eq!(
        crate::keymap::act(KeyEvent::new(KeyCode::Char('e'), KeyModifiers::CONTROL)),
        None,
        "C-e is not e"
    );
}

#[test]
fn the_readme_names_every_key_the_help_does() {
    let readme = include_str!("../../README.md");
    let said = crate::keymap::KEYS
        .iter()
        .filter(|b| !b.help.is_empty() || !b.foot.is_empty());
    for b in said {
        for key in b.name.split(' ') {
            assert!(readme.contains(&format!("`{key}`")), "`{key}` in README");
        }
    }
}

#[test]
fn the_foot_and_the_help_come_from_the_table() {
    let mut app = demo();
    let foot = app::screen(&mut app, 120, 30);
    for h in crate::keymap::foot(crate::keymap::Pane::List) {
        let hint = format!(" {} {}", h.key, h.what);
        assert!(foot.contains(&hint), "{hint} in {foot}");
    }
    press(&mut app, &["?"]);
    let help = app::screen(&mut app, 100, 160);
    for (name, _) in crate::keymap::help() {
        assert!(help.contains(name), "{name} in {help}");
    }
}

#[test]
fn the_wheel_moves_in_the_dialog_open_and_not_behind_it() {
    let mut app = demo();
    let turing = app
        .shown
        .iter()
        .position(|m| m.from == "Alan Turing")
        .unwrap();
    app.list.select(Some(turing));
    press(&mut app, &["<enter>", "o"]);
    let _ = app::screen(&mut app, 120, 40);
    for _ in 0..3 {
        app.mouse(mouse(MouseEventKind::ScrollDown, 1, 1));
    }
    assert_eq!(
        app.dialog.files().expect("listed").at,
        3,
        "the last of four"
    );
    press(&mut app, &["<esc>"]);

    // A question asked, the list behind it stays where it is.
    app.dialog = app::Dialog::Rename(0, "Demo".to_string());
    let before = app.list.selected();
    let _ = app::screen(&mut app, 120, 40);
    app.mouse(mouse(MouseEventKind::ScrollDown, 60, 10));
    assert_eq!(app.list.selected(), before);
    assert!(matches!(app.dialog, app::Dialog::Rename(..)));
}

#[test]
fn space_pages_the_message_and_leaves_the_sidebar_alone() {
    let mut app = demo();
    press(&mut app, &["h"]);
    let (row, picked) = (app.row, app.list.selected());
    press(&mut app, &[" "]);
    assert_eq!(app.row, row, "space does not move the folders");
    assert_eq!(app.list.selected(), picked, "nor the messages");
    press(&mut app, &["<pgdn>"]);
    assert_ne!(app.row, row, "the page key does");
}

#[test]
fn c_x_c_c_quits_as_emacs_does_and_q_alone_does_not() {
    let mut app = demo();
    press(&mut app, &["q"]);
    assert!(!app.quit, "q alone quits no more");
    assert!(app.status.contains("C-x C-c quits"), "{}", app.status);
    // C-x, then anything but C-c, goes on as before.
    press(&mut app, &["<c-x>"]);
    assert!(app.status.contains("C-x"), "{}", app.status);
    press(&mut app, &["j"]);
    assert!(!app.quit);
    // C-c alone, as compose's C-c C-c might be pressed once it is gone, no
    // more.
    press(&mut app, &["<c-c>", "<c-c>"]);
    assert!(!app.quit, "C-c alone quits no more");
    assert!(app.status.contains("C-x C-c quits"), "{}", app.status);
    press(&mut app, &["<c-x>", "<c-c>"]);
    assert!(app.quit);
    let mut app = demo();
    let screen = app::screen(&mut app, 140, 40);
    assert!(screen.contains("C-x C-c quit"), "{screen}");
}

#[test]
fn emacs_keys_move_in_the_list_and_the_map_as_in_the_inbox_looked_through() {
    use ratatui::crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
    let mut app = demo();
    let at = |app: &App| app.list.selected();
    let first = at(&app);
    press(&mut app, &["<c-n>", "<c-n>"]);
    assert_eq!(at(&app), first.map(|f| f + 2));
    press(&mut app, &["<c-p>"]);
    assert_eq!(at(&app), first.map(|f| f + 1));
    assert!(app.compose.is_none(), "C-n begins no mail");
    // M-> and M-<, Shift held for them or not, as terminals say it.
    for held in [KeyModifiers::ALT, KeyModifiers::ALT | KeyModifiers::SHIFT] {
        app.key(KeyEvent::new(KeyCode::Char('>'), held));
        assert_eq!(at(&app), Some(app.shown.len() - 1), "{held:?}");
        app.key(KeyEvent::new(KeyCode::Char('<'), held));
        assert_eq!(at(&app), Some(0), "{held:?}");
    }
    // C-f forwards no more: it reads the one picked, as l does.
    press(&mut app, &["<c-f>"]);
    assert!(app.compose.is_none());
    assert_eq!(app.focus, app::Focus::Reader);
    // On the map, C-n goes to the nearest message down, as j does.
    let mut app = demo_map();
    let _ = app::screen(&mut app, 140, 40);
    press(&mut app, &["0"]);
    let _ = app::screen(&mut app, 140, 40);
    let before = app.map.picked();
    press(&mut app, &["j"]);
    let by_j = app.map.picked();
    app.map.cursor = before.map(|i| app.map.snapshot.as_ref().unwrap().dots[i].key.clone());
    press(&mut app, &["<c-n>"]);
    assert_eq!(app.map.picked(), by_j);
}

#[test]
fn a_narrow_foot_leaves_hints_out_but_help_and_quit() {
    let mut app = demo();
    let screen = app::screen(&mut app, 40, 20);
    assert!(
        screen.contains(" ? help") && screen.contains(" C-x C-c quit"),
        "{screen}"
    );
    assert!(!screen.contains("new mail"), "{screen}");
    // Writing, the foot fits in its two lines too.
    press(&mut app, &["c"]);
    let screen = app::screen(&mut app, 40, 20);
    let lines: Vec<&str> = screen.lines().collect();
    assert!(lines[lines.len() - 3].contains("Compose"), "{screen}");
    assert!(lines[lines.len() - 2].contains("send"), "{screen}");
}

// ---- the colours ----

/// WCAG's contrast of two colours, 1 to 21; the terminal's own dark
/// ground as the theme takes it.
pub(super) fn contrast(a: ratatui::style::Color, b: ratatui::style::Color) -> f64 {
    use ratatui::style::Color;
    let luminance = |c: Color| {
        let (r, g, b) = match c {
            Color::Rgb(r, g, b) => (r, g, b),
            Color::Reset => crate::theme::TERMINAL_DARK,
            other => panic!("{other:?} is not a colour to measure"),
        };
        let lin = |v: u8| {
            let v = f64::from(v) / 255.0;
            if v <= 0.04045 {
                v / 12.92
            } else {
                ((v + 0.055) / 1.055).powf(2.4)
            }
        };
        0.2126 * lin(r) + 0.7152 * lin(g) + 0.0722 * lin(b)
    };
    let (x, y) = (luminance(a), luminance(b));
    (x.max(y) + 0.05) / (x.min(y) + 0.05)
}

#[test]
fn every_colour_reads_on_every_ground_it_is_drawn_on() {
    use crate::theme::{Name, Theme};
    use clap::ValueEnum;
    for &name in Name::value_variants() {
        let t = Theme::of(name);
        let accounts = t.accounts().iter().map(|&c| ("account", c, 4.5));
        let inks = [
            ("title", t.title, 7.0),
            ("text", t.text, 7.0),
            ("soft", t.soft, 4.5),
            ("dim", t.dim, 4.5),
            // Signatures and notes are faint too: text, not only icons.
            ("faint", t.faint, 4.5),
            ("accent", t.accent, 4.5),
            ("flag", t.flag, 4.5),
            ("error", t.error, 4.5),
        ];
        let grounds = [
            ("page", t.page),
            ("side", t.side),
            ("field", t.field),
            ("focus", t.focus),
            ("row", t.row),
        ];
        for (ink, colour, need) in inks.into_iter().chain(accounts) {
            for (ground, behind) in grounds {
                let got = contrast(colour, behind);
                assert!(
                    got >= need,
                    "{name:?}: {ink} on {ground} is {got:.1}:1, under {need}:1"
                );
            }
        }
        // Labels drawn on a colour: the button pressed with ⏎, and the
        // mode line's name of the message being written.
        for (what, ink, behind) in [
            ("a button", t.on_accent, t.accent),
            ("Compose", t.side, t.title),
        ] {
            let got = contrast(ink, behind);
            assert!(got >= 4.5, "{name:?}: {what} reads {got:.1}:1");
        }
        // Each step down reads less than the one above it.
        let steps = [t.title, t.text, t.soft, t.dim, t.faint].map(|c| contrast(c, t.page));
        assert!(steps.is_sorted_by(|a, b| a > b), "{name:?}: {steps:?}");
        // Grounds that must be told apart: the panes, the picked row in
        // the pane with the focus and in the others, and a rule under it.
        for (what, a, b) in [
            ("side and page", t.side, t.page),
            ("side and row", t.side, t.row),
            ("focus and row", t.focus, t.row),
            ("rule and focus", t.rule, t.focus),
            ("rule and page", t.rule, t.page),
        ] {
            let got = contrast(a, b);
            assert!(got >= 1.12, "{name:?}: {what} are {got:.2}:1 apart");
        }
    }
}

#[test]
fn unread_a_flag_and_the_accounts_are_told_apart_by_hue() {
    // Blue for unread, orange for a flag, as every common colour blindness
    // tells them apart; the accounts in neither.
    use crate::theme::{Name, Theme};
    use clap::ValueEnum;
    use ratatui::style::Color;
    let hue = |c: Color| {
        let Color::Rgb(r, g, b) = c else {
            panic!("{c:?} is not a colour");
        };
        let (r, g, b) = (f64::from(r), f64::from(g), f64::from(b));
        let (max, min) = (r.max(g).max(b), r.min(g).min(b));
        let h = if max == r {
            (g - b) / (max - min)
        } else if max == g {
            2.0 + (b - r) / (max - min)
        } else {
            4.0 + (r - g) / (max - min)
        };
        (h * 60.0).rem_euclid(360.0)
    };
    let apart = |a: f64, b: f64| {
        let d = (a - b).abs();
        d.min(360.0 - d)
    };
    for &name in Name::value_variants() {
        let t = Theme::of(name);
        let (unread, flag) = (hue(t.accent), hue(t.flag));
        assert!(
            (190.0..230.0).contains(&unread),
            "{name:?}: unread {unread}"
        );
        assert!((20.0..40.0).contains(&flag), "{name:?}: flag {flag}");
        for (i, &c) in t.accounts().iter().enumerate() {
            let h = hue(c);
            for (what, other) in [("unread", unread), ("flag", flag)] {
                let d = apart(h, other);
                assert!(d >= 30.0, "{name:?}: account {i} is {d} from {what}");
            }
        }
    }
}

// ---- the map's index: its journals and its key ----

/// A record of the account `a@b.c`, as fetched from INBOX, known by `id`.
pub(super) fn record(id: &str, subject: &str) -> index::Record {
    let indexed = index::record::Indexed {
        uid: 1,
        size: 100,
        raw: raw(
            "Ada Lovelace <ada@analytical.example>",
            subject,
            "",
            "Hello.\r\n",
        ),
        ..Default::default()
    };
    let mut r = index::Record::of("a@b.c", "INBOX", 7, &indexed, 1_000);
    r.key.id = id.to_string();
    r
}

pub(super) fn identity() -> age::x25519::Identity {
    age::x25519::Identity::generate()
}

/// The journals' files in `dir`: all of it but the lock held as one is
/// written to.
pub(super) fn journal_files(dir: &std::path::Path) -> Vec<std::path::PathBuf> {
    std::fs::read_dir(dir)
        .unwrap()
        .map(|e| e.unwrap().path())
        .filter(|p| p.file_name().is_some_and(|n| n != "lock"))
        .collect()
}

#[test]
fn the_journals_give_back_what_was_written() {
    let dir = tempfile::tempdir().unwrap();
    let id = identity();
    let (a, b) = (record("a", "Notes on the engine"), record("b", "Photo 51"));
    {
        let mut index = Index::open_with(dir.path(), &id);
        index
            .cache(vec![
                Cached::Upsert(Box::new(a.clone())),
                Cached::Upsert(Box::new(b.clone())),
                Cached::Flags {
                    key: b.key.clone(),
                    flags: mail::Flags {
                        seen: true,
                        ..Default::default()
                    },
                },
            ])
            .unwrap();
        index
            .keep(vec![
                Kept::Label {
                    key: a.key.clone(),
                    label: "Grant".into(),
                    on: true,
                },
                Kept::Layout {
                    lens: Lens::Both,
                    at: vec![(a.key.clone(), 1.5, -2.0)],
                },
                Kept::Looked { at: 42 },
            ])
            .unwrap();
        index.flush().unwrap();
    }
    let index = Index::open_with(dir.path(), &id);
    assert!(index.fault.is_none(), "{:?}", index.fault);
    assert_eq!(index.records.len(), 2);
    assert_eq!(index.records[&a.key], a);
    assert!(index.records[&b.key].flags.seen);
    assert_eq!(index.keep.labels[&a.key], ["Grant"]);
    assert_eq!(index.keep.layouts[&Lens::Both][&a.key], (1.5, -2.0));
    assert_eq!(index.keep.looked, 42);
    // Encrypted: nothing of the mail is in the files.
    for path in journal_files(dir.path()) {
        let bytes = std::fs::read(path).unwrap();
        let text = String::from_utf8_lossy(&bytes);
        assert!(!text.contains("Photo") && !text.contains("Grant"));
        assert!(text.starts_with("age-encryption.org/v1"));
    }
    // Another key reads nothing, and says so.
    drop(index);
    let other = Index::open_with(dir.path(), &identity());
    assert!(other.records.is_empty());
    assert!(other.keep_fault().is_some());
}

#[test]
fn new_mail_is_written_with_the_public_key_alone() {
    let dir = tempfile::tempdir().unwrap();
    let id = identity();
    let a = record("a", "Notes on the engine");
    {
        let mut index = Index::open_with(dir.path(), &id);
        index
            .cache(vec![Cached::Upsert(Box::new(a.clone()))])
            .unwrap();
    }
    let b = record("b", "Photo 51");
    {
        let mut writer = Index::writer(dir.path(), id.to_public());
        writer
            .cache(vec![Cached::Upsert(Box::new(b.clone()))])
            .unwrap();
        writer.flush().unwrap();
    }
    let index = Index::open_with(dir.path(), &id);
    assert_eq!(index.records.len(), 2, "{:?}", index.fault);
    assert_eq!(index.records[&b.key].subject, "Photo 51");
}

#[test]
fn a_base_written_and_its_logs_not_yet_removed_is_read_once() {
    use crate::index::journal::Journal;
    let dir = tempfile::tempdir().unwrap();
    let aside = tempfile::tempdir().unwrap();
    let id = identity();
    let open = || Journal::<u32>::open(dir.path(), "cache", id.to_public(), Some(&id));
    {
        let (mut j, _) = open();
        for n in 0..3 {
            j.push(n).unwrap();
            j.flush().unwrap();
        }
        assert_eq!(j.logs(), 3);
    }
    let copy = |from: &std::path::Path, to: &std::path::Path| {
        for entry in std::fs::read_dir(from).unwrap() {
            let path = entry.unwrap().path();
            std::fs::copy(&path, to.join(path.file_name().unwrap())).unwrap();
        }
    };
    copy(dir.path(), aside.path());
    {
        let (mut j, all) = open();
        assert_eq!(all, [0, 1, 2]);
        j.compact(&all).unwrap();
        assert_eq!(j.logs(), 0);
    }
    // As if letterbox stopped once the base was written, before the files
    // it takes the place of were removed.
    copy(aside.path(), dir.path());
    let (mut j, all) = open();
    assert_eq!(all, [0, 1, 2], "the new base alone");
    j.push(3).unwrap();
    j.flush().unwrap();
    drop(j);
    assert_eq!(open().1, [0, 1, 2, 3]);
}

#[test]
fn a_journal_with_many_logs_is_written_again_as_one_base() {
    let dir = tempfile::tempdir().unwrap();
    let id = identity();
    let many = crate::index::journal::LOGS + 1;
    {
        let mut index = Index::open_with(dir.path(), &id);
        for n in 0..many {
            let r = record(&n.to_string(), "Photo 51");
            index.cache(vec![Cached::Upsert(Box::new(r))]).unwrap();
            index.flush().unwrap();
        }
    }
    let files = || journal_files(dir.path()).len();
    assert_eq!(files(), many);
    let index = Index::open_with(dir.path(), &id);
    assert_eq!(index.records.len(), many);
    assert_eq!(files(), 1, "one base, the cache's; keep has nothing");
    drop(index);
    assert_eq!(Index::open_with(dir.path(), &id).records.len(), many);
}

/// Spoil the first file of `name` in `dir`, as a disk might; where it is.
pub(super) fn spoil(dir: &std::path::Path, name: &str) -> PathBuf {
    let path = std::fs::read_dir(dir)
        .unwrap()
        .map(|e| e.unwrap().path())
        .find(|p| p.file_name().unwrap().to_string_lossy().starts_with(name))
        .unwrap();
    let mut bytes = std::fs::read(&path).unwrap();
    let at = bytes.len() - 5;
    bytes[at] ^= 0xff;
    std::fs::write(&path, &bytes).unwrap();
    path
}

#[test]
fn a_cache_that_cannot_be_read_is_begun_again() {
    let dir = tempfile::tempdir().unwrap();
    let id = identity();
    {
        let mut index = Index::open_with(dir.path(), &id);
        let r = record("a", "Photo 51");
        index.cache(vec![Cached::Upsert(Box::new(r))]).unwrap();
    }
    spoil(dir.path(), "cache.");
    let index = Index::open_with(dir.path(), &id);
    assert!(index.records.is_empty());
    let fault = index.fault.clone().unwrap();
    assert!(fault.contains("made again from the servers"), "{fault}");
    drop(index);
    let index = Index::open_with(dir.path(), &id);
    assert!(index.fault.is_none(), "{:?}", index.fault);
}

#[test]
fn a_keep_that_cannot_be_read_is_not_written_over() {
    let dir = tempfile::tempdir().unwrap();
    let id = identity();
    let key = record("a", "Photo 51").key;
    let label = |on| Kept::Label {
        key: key.clone(),
        label: "Grant".into(),
        on,
    };
    {
        let mut index = Index::open_with(dir.path(), &id);
        index.keep(vec![label(true)]).unwrap();
    }
    let path = spoil(dir.path(), "keep.");
    let before = std::fs::read(&path).unwrap();
    let mut index = Index::open_with(dir.path(), &id);
    assert!(index.keep_fault().is_some());
    let e = index.keep(vec![label(false)]).unwrap_err();
    assert!(e.to_string().contains("cannot read"), "{e}");
    index.flush().unwrap();
    drop(index);
    assert_eq!(std::fs::read(&path).unwrap(), before);
    assert_eq!(journal_files(dir.path()).len(), 1);
}

/// Where a test keeps the map's key: here, the system's store left alone.
pub(super) static KEPT_KEY: Mutex<Option<(Keeper, String)>> = Mutex::new(None);

pub(super) fn keep_key(keeper: Keeper, host: &str, user: &str, secret: &str) -> anyhow::Result<()> {
    assert_eq!((host, user), ("map", "identity"));
    *KEPT_KEY.lock().unwrap() = Some((keeper, secret.to_string()));
    Ok(())
}

pub(super) fn find_key(keeper: Keeper, _: &str, _: &str) -> anyhow::Result<String> {
    match KEPT_KEY.lock().unwrap().clone() {
        Some((k, secret)) if k == keeper => Ok(secret),
        _ => anyhow::bail!("no such key"),
    }
}

pub(super) fn refuse_key(_: Keeper, _: &str, _: &str, _: &str) -> anyhow::Result<()> {
    anyhow::bail!("the store is closed")
}

#[test]
fn the_maps_key_is_kept_before_it_is_named() {
    use crate::index::key;
    let dir = tempfile::tempdir().unwrap();
    // Nowhere to keep it: no map.toml names a key that is not kept.
    let e = Index::open(dir.path(), || vec![Keeper::Keyring], find_key, refuse_key)
        .err()
        .unwrap();
    assert!(e.contains("the store is closed"), "{e}");
    assert!(key::card(dir.path()).unwrap().is_none());

    let r = record("a", "Photo 51");
    {
        let mut index =
            Index::open(dir.path(), || vec![Keeper::Keyring], find_key, keep_key).unwrap();
        index
            .cache(vec![Cached::Upsert(Box::new(r.clone()))])
            .unwrap();
    }
    let card = key::card(dir.path()).unwrap().unwrap();
    assert_eq!((card.version, card.keeper), (key::VERSION, Keeper::Keyring));
    let secret = KEPT_KEY.lock().unwrap().clone().unwrap().1;
    assert!(secret.starts_with("AGE-SECRET-KEY-"));
    let written = std::fs::read_to_string(dir.path().join("map.toml")).unwrap();
    assert!(!written.contains(&secret), "{written}");
    // Opened again, the key is found where the card says, and reads it.
    let index = Index::open(dir.path(), || vec![Keeper::Vault], find_key, refuse_key).unwrap();
    assert_eq!(index.records[&r.key].subject, "Photo 51");
    let found = key::identity(&card, find_key).unwrap();
    assert!(key::export(&found).contains(&secret));
}

// ---- the map's sync ----

/// One pass of the sync over `store`, into `index`, as at the time `now`.
pub(super) fn sync_at(
    store: &mut dyn Store,
    index: &Mutex<Index>,
    now: i64,
) -> index::sync::Passed {
    let stop = std::sync::atomic::AtomicBool::new(false);
    index::sync::pass(store, index, now, &stop, &mut |_| {}).unwrap()
}

pub(super) fn sync_once(store: &mut dyn Store, index: &Mutex<Index>) -> index::sync::Passed {
    sync_at(store, index, 1_000_000)
}

/// The folders the index places each of `account`'s messages in, by
/// subject.
pub(super) fn places(index: &Mutex<Index>) -> std::collections::BTreeMap<String, Vec<String>> {
    index
        .lock()
        .unwrap()
        .records
        .values()
        .map(|r| {
            (
                r.subject.clone(),
                r.places.iter().map(|p| p.folder.clone()).collect(),
            )
        })
        .collect()
}

/// The UID sets of the messages fetched whole for the index, as the
/// server was asked for them, in turn.
pub(super) fn index_fetches(boxes: &Boxes) -> Vec<String> {
    boxes
        .lock()
        .unwrap()
        .said
        .iter()
        .filter(|s| s.starts_with("UID FETCH") && s.contains("LIST-ID"))
        .map(|s| s.split(' ').nth(2).unwrap_or_default().to_string())
        .collect()
}

#[test]
fn the_sync_reads_the_inbox_first_newest_first_a_hundred_at_a_time() {
    let (account, boxes) = serve_boxes(boxes_in(Mode::Keywords));
    let mut uids = Vec::new();
    for n in 0..150 {
        uids.push(add_listed(&boxes, "INBOX", &[], &format!("Build {n}")));
    }
    add_listed(&boxes, "Archive", &["\\Seen"], "Old news");
    add_listed(&boxes, "Drafts", &[], "Unfinished");
    add_listed(&boxes, "Deleted Items", &[], "Thrown away");
    let index = Mutex::new(Index::memory());
    let mut imap = Imap::new(account);
    let passed = sync_once(&mut imap, &index);
    assert!(passed.complete);
    let fetches = index_fetches(&boxes);
    let newest = crate::imap::uid_set(&uids[50..]);
    assert_eq!(fetches[0], newest, "the newest hundred of the Inbox first");
    assert_eq!(fetches.len(), 3, "{fetches:?}");
    let at = places(&index);
    assert_eq!(at.len(), 3 + 150 + 1, "{:?}", at.keys());
    assert_eq!(at["Old news"], ["Archive"]);
    assert_eq!(at["Photo 51"], ["INBOX"]);
    assert!(!at.contains_key("Unfinished") && !at.contains_key("Thrown away"));

    // Again, nothing new: nothing is fetched whole.
    sync_once(&mut imap, &index);
    assert_eq!(index_fetches(&boxes).len(), 3);
}

#[test]
fn new_mail_and_mail_gone_are_seen_by_the_next_pass() {
    let (account, boxes) = serve_boxes(boxes_in(Mode::Keywords));
    let index = Mutex::new(Index::memory());
    let mut imap = Imap::new(account);
    sync_once(&mut imap, &index);
    let new = add_listed(&boxes, "INBOX", &[], "PR #38 merged");
    boxes.lock().unwrap()[0].letters.retain(|l| l.uid != 11);
    sync_at(&mut imap, &index, 2_000_000);
    assert_eq!(index_fetches(&boxes).last().unwrap(), &new.to_string());
    let at = places(&index);
    assert!(at.contains_key("PR #38 merged"));
    assert!(!at.contains_key("Photo 51"), "forgotten once gone");
    let index = index.lock().unwrap();
    let pr = index
        .records
        .values()
        .find(|r| r.subject == "PR #38 merged");
    assert_eq!(pr.unwrap().first_seen, 2_000_000);
}

#[test]
fn a_message_moved_keeps_its_key_and_its_first_sight() {
    let (account, _) = serve_boxes(boxes_in(Mode::Keywords));
    let index = Mutex::new(Index::memory());
    let mut imap = Imap::new(account);
    sync_once(&mut imap, &index);
    let key = |index: &Mutex<Index>| {
        let index = index.lock().unwrap();
        let r = index
            .records
            .values()
            .find(|r| r.subject == "Photo 51")
            .unwrap();
        (r.key.clone(), r.first_seen, r.places.clone())
    };
    let (before, seen, _) = key(&index);
    imap.relocate("INBOX", 11, "Archive", "").unwrap();
    sync_at(&mut imap, &index, 3_000_000);
    let (after, still, places) = key(&index);
    assert_eq!((after, still), (before, seen));
    assert_eq!(places.len(), 1);
    assert_eq!(places[0].folder, "Archive");
    assert_ne!(places[0].uid, 11);
    assert_eq!(index.lock().unwrap().records.len(), 3);
}

#[test]
fn a_folder_whose_uidvalidity_changed_is_read_again() {
    let (account, boxes) = serve_boxes(boxes_in(Mode::Keywords));
    let index = Mutex::new(Index::memory());
    let mut imap = Imap::new(account);
    sync_once(&mut imap, &index);
    {
        let mut fake = boxes.lock().unwrap();
        let inbox = &mut fake[0];
        inbox.validity = 8;
        for (i, l) in inbox.letters.iter_mut().enumerate() {
            l.uid = 100 + i as u32;
        }
        inbox.next = 200;
    }
    sync_once(&mut imap, &index);
    let index = index.lock().unwrap();
    assert_eq!(index.records.len(), 3);
    for r in index.records.values() {
        assert_eq!(r.places.len(), 1);
        assert_eq!((r.places[0].validity, r.places[0].uid >= 100), (8, true));
    }
}

#[test]
fn flags_changed_on_the_server_are_read_by_mod_sequence_or_again() {
    for mode in [Mode::Keywords, Mode::Folders] {
        let (account, boxes) = serve_boxes(boxes_in(mode));
        let index = Mutex::new(Index::memory());
        let mut imap = Imap::new(account);
        sync_once(&mut imap, &index);
        {
            let mut fake = boxes.lock().unwrap();
            let seq = fake.tick();
            let emmy = &mut fake[0].letters[2];
            emmy.flags.push("\\Seen".into());
            emmy.modseq = seq;
        }
        sync_at(&mut imap, &index, 1_000_100);
        let index = index.lock().unwrap();
        let emmy = index
            .records
            .values()
            .find(|r| r.address == "noether@goettingen.example")
            .unwrap();
        assert!(emmy.flags.seen, "{mode:?}");
        let said = boxes.lock().unwrap().said.join("\n");
        assert_eq!(
            said.contains("CHANGEDSINCE"),
            mode == Mode::Keywords,
            "{said}"
        );
    }
}

#[test]
fn gmail_is_read_through_all_mail_alone() {
    let (account, boxes) = serve_boxes(gmail_boxes());
    let index = Mutex::new(Index::memory());
    let mut imap = Imap::new(account);
    assert!(sync_once(&mut imap, &index).complete);
    let said = boxes.lock().unwrap().said.clone();
    let selected: Vec<&String> = said.iter().filter(|s| s.starts_with("SELECT")).collect();
    assert!(
        selected.iter().all(|s| s.contains("All Mail")),
        "{selected:?}"
    );
    let index = index.lock().unwrap();
    assert_eq!(index.records.len(), 3);
    let aims = index
        .records
        .values()
        .find(|r| r.subject == "Aims page, third pass")
        .unwrap();
    assert_eq!(aims.key.id, "gm:9021");
    assert_eq!(aims.server, ["\\Inbox", "Projects/Grant"]);
}

#[test]
fn the_demo_is_mapped_from_a_twin_that_sees_its_mail() {
    let demo = Demo::new();
    let mut twin = demo.twin().unwrap();
    let index = Mutex::new(Index::memory());
    sync_once(twin.as_mut(), &index);
    // The Inbox, Archive, Sent and Projects/Grant; not Drafts.
    assert_eq!(index.lock().unwrap().records.len(), INBOX + ARCHIVE + 4 + 4);
    let mut demo = demo;
    demo.relocate("INBOX", 1, "Trash", "").unwrap();
    sync_once(twin.as_mut(), &index);
    assert_eq!(
        index.lock().unwrap().records.len(),
        INBOX + ARCHIVE + 4 + 4 - 1
    );
}

#[test]
fn the_app_maps_every_account_as_it_starts() {
    let app = demo();
    assert_eq!(
        app.index.lock().unwrap().records.len(),
        INBOX + ARCHIVE + 4 + 4
    );
    assert!(app.mapping.is_empty(), "{}", app.mapping);
}
