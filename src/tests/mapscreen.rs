//! The map on the screen, what is marked on it, laid out again, and its
//! model on mail made up of known topics.

use super::*;

// ---- the map, on the screen ----

/// Read the message picked on the map: its cluster gone into, and its
/// parts, down to it.
pub(super) fn read_on_map(app: &mut App) {
    for _ in 0..8 {
        press(app, &["<enter>"]);
    }
}

/// The demo, its map shown.
pub(super) fn demo_map() -> App {
    let mut app = demo();
    press(&mut app, &["<m-g>"]);
    app
}

#[test]
fn the_map_and_the_list_are_clicked_at_the_top_or_super_g() {
    let mut app = demo();
    let map = crate::keymap::key_named("M-g").unwrap();
    let list = crate::keymap::key_named("M-l").unwrap();
    click_at(&mut app, app::Target::Key(map));
    assert_eq!(app.map.mode, crate::map::Mode::Map);
    let screen = app::screen(&mut app, 120, 40);
    assert!(
        screen.lines().next().unwrap().contains(" List  Map "),
        "{screen}"
    );
    click_at(&mut app, app::Target::Key(list));
    assert_eq!(app.map.mode, crate::map::Mode::List);
    // ⌘G or Super+G, as a terminal that says Super says it.
    app.key(KeyEvent::new(KeyCode::Char('g'), KeyModifiers::SUPER));
    app.settle(Duration::from_secs(5));
    assert_eq!(app.map.mode, crate::map::Mode::Map);
    // Ctrl+Shift+G is no longer the map's.
    press(&mut app, &["<m-l>"]);
    app.key(KeyEvent::new(
        KeyCode::Char('G'),
        KeyModifiers::CONTROL | KeyModifiers::SHIFT,
    ));
    assert_eq!(app.map.mode, crate::map::Mode::List);
}

/// The subject of the message picked on the map.
pub(super) fn picked_subject(app: &App) -> String {
    let s = app.map.snapshot.as_ref().unwrap();
    s.dots[app.map.picked().expect("one picked")]
        .subject
        .clone()
}

#[test]
fn the_map_lays_out_every_message_with_its_clusters() {
    let mut app = demo_map();
    assert_eq!(app.map.mode, crate::map::Mode::Map);
    let s = app.map.snapshot.clone().expect("drawn");
    assert_eq!(s.dots.len(), INBOX + ARCHIVE + 4 + 4);
    assert!(s.at.is_some(), "laid out");
    // The layout is kept, to be the same next time.
    assert_eq!(
        app.index.lock().unwrap().keep.layouts[&Lens::Both].len(),
        s.dots.len()
    );
    let screen = app::screen(&mut app, 150, 45);
    assert!(
        screen.contains("Map · its clusters · words and people"),
        "{screen}"
    );
    assert!(screen.contains("◆ Grant"), "{screen}");
    // No sidebar beside it: what it would say, the map says itself.
    assert!(
        !screen.contains(" DEMO") && !screen.contains("Every cluster"),
        "{screen}"
    );
    assert!(screen.contains("% faithful"), "{screen}");
    assert!(screen.contains("blocks lie by those alike"), "{screen}");
    // The newest new message is picked, named beside it, and read beside
    // the map once the cursor rests.
    assert_eq!(
        picked_subject(&app),
        "Notes on the engine, and a table of Bernoulli numbers"
    );
    assert!(
        screen.contains("Ada Lovelace — Notes on the engine"),
        "{screen}"
    );
    let foot: Vec<&str> = screen.lines().rev().take(2).collect();
    let foot = foot.join(" ");
    assert!(
        foot.contains("/ search") && foot.contains("m move to"),
        "{foot}"
    );
}

#[test]
fn the_map_is_narrowed_to_a_cluster_and_listed() {
    let mut app = demo_map();
    press(&mut app, &["]"]);
    let crate::map::Focused::Cluster(id) = app.map.focused else {
        panic!("not narrowed: {:?}", app.map.focused);
    };
    let group = app
        .map
        .snapshot
        .as_ref()
        .unwrap()
        .group(id)
        .unwrap()
        .clone();
    assert!(group.members.contains(&app.map.picked().unwrap()));
    press(&mut app, &["<esc>"]);
    assert_eq!(app.map.focused, crate::map::Focused::All);
    // Grant's anchor clicked narrows the map to it, and the list lists it.
    let grant = app
        .map
        .snapshot
        .as_ref()
        .unwrap()
        .groups
        .iter()
        .find(|g| g.label.as_deref() == Some("Grant"))
        .expect("Grant on the map")
        .id;
    click_at(&mut app, app::Target::Anchor(grant));
    assert_eq!(app.map.focused, crate::map::Focused::Cluster(grant));
    press(&mut app, &["<m-l>"]);
    assert_eq!(app.map.mode, crate::map::Mode::List);
    assert_eq!(app.folder_name(), "Grant");
    let subjects: Vec<&str> = app.shown.iter().map(|m| m.subject.as_str()).collect();
    assert!(subjects.contains(&"Aims page, third pass"), "{subjects:?}");
    // Read from the list, it is read on its own account and folder.
    let aims = app
        .shown
        .iter()
        .position(|m| m.subject == "Aims page, third pass")
        .unwrap();
    app.list.select(Some(aims));
    press(&mut app, &["<enter>"]);
    assert_eq!(app.message().unwrap().subject, "Aims page, third pass");
    press(&mut app, &["<m-g>"]);
    assert_eq!(app.map.mode, crate::map::Mode::Map);
}

#[test]
fn the_lens_the_time_and_read_mail_are_turned() {
    let mut app = demo_map();
    let all = app.map.shown(jiff::Timestamp::now().as_second()).len();
    // From the view menu, which stays open to change more.
    press(&mut app, &[",", "w"]);
    assert!(matches!(app.dialog, app::Dialog::View));
    assert_eq!(app.map.lens, Lens::Words);
    assert_eq!(app.map.snapshot.as_ref().unwrap().lens, Lens::Words);
    assert!(
        app.index
            .lock()
            .unwrap()
            .keep
            .layouts
            .contains_key(&Lens::Words)
    );
    press(&mut app, &["w", "w"]);
    assert_eq!(app.map.lens, Lens::Both);
    press(&mut app, &["t"]);
    let now = jiff::Timestamp::now().as_second();
    let week = app.map.shown(now).len();
    assert!(week < all, "{week} of {all}");
    press(&mut app, &["t", "t"]);
    press(&mut app, &["r"]);
    let unread = app.map.shown(now).len();
    assert_eq!(unread, UNREAD as usize);
    press(&mut app, &["r", "n"]);
    assert!(app.map.new_only);
    press(&mut app, &["s"]);
    assert!(app.map.hide_sent);
    // Any other key closes it; l lays the map out again.
    press(&mut app, &["<esc>"]);
    assert!(matches!(app.dialog, app::Dialog::Closed));
    press(&mut app, &[",", "l"]);
    assert!(matches!(app.dialog, app::Dialog::Closed));
    assert!(app.map.preview.is_some(), "{}", app.status);
}

#[test]
fn the_map_fits_any_screen_light_or_dark() {
    use crate::theme::{Name, Theme};
    for name in [Name::Light, Name::Dark] {
        for width in [16, 33, 69, 70, 119, 120, 160] {
            for height in [6, 9, 24, 48] {
                let mut app = demo_map();
                app.theme = Theme::of(name);
                let screen = app::screen(&mut app, width, height);
                assert_eq!(screen.lines().count(), height as usize);
                let _ = app::screen_svg(&mut app, width, height);
            }
        }
    }
    let mut app = demo_map();
    let narrow = app::screen(&mut app, 60, 24);
    assert!(
        narrow.contains("Map ·") && !narrow.contains("DEMO"),
        "{narrow}"
    );
}

#[test]
fn a_message_in_two_accounts_is_two_dots_marked_twins() {
    let mut app = two_accounts();
    press(&mut app, &["<m-g>"]);
    let s = app.map.snapshot.clone().unwrap();
    assert_eq!(s.dots.len(), 2 * (INBOX + ARCHIVE + 4 + 4) + 1);
    let photo: Vec<&crate::map::Dot> = s.dots.iter().filter(|d| d.subject == "Photo 51").collect();
    assert_eq!(photo.len(), 2);
    assert!(photo.iter().all(|d| d.twin));
    assert!(
        !s.dots
            .iter()
            .find(|d| d.subject == "Newest of all")
            .unwrap()
            .twin
    );
}

#[test]
fn the_map_says_why_it_is_not_there() {
    let mut app = App::with_index(
        vec![("Demo".to_string(), Box::new(Demo::new()) as Box<dyn Store>)],
        PathBuf::from("/nowhere/accounts.toml"),
        Err("the map's key would be kept in the vault, which is locked".to_string()),
    );
    app.ledger = scratch_ledger();
    app.settle(Duration::from_secs(5));
    press(&mut app, &["<m-g>"]);
    assert!(
        app.status.contains("vault, which is locked"),
        "{}",
        app.status
    );
    assert_eq!(app.map.mode, crate::map::Mode::List);
}

#[test]
fn the_map_is_begun_only_once_it_is_asked_for_and_agreed_to() {
    let mut app = App::with_index(
        vec![("Demo".to_string(), Box::new(Demo::new()) as Box<dyn Store>)],
        PathBuf::from("/nowhere/accounts.toml"),
        Ok(None),
    );
    app.open_map = || Ok(Index::memory());
    app.ledger = scratch_ledger();
    app.settle(Duration::from_secs(5));
    // Nothing is read for it until then.
    assert!(app.index.lock().unwrap().records.is_empty());
    press(&mut app, &["<m-g>"]);
    assert!(matches!(app.dialog, app::Dialog::StartMap));
    let screen = app::screen(&mut app, 120, 40);
    assert!(
        screen.contains("Begin the map?") && screen.contains("encrypted"),
        "{screen}"
    );
    // Not now: nothing begun, and asked again.
    press(&mut app, &["n"]);
    assert_eq!(app.map.mode, crate::map::Mode::List);
    assert!(app.status.contains("asks again"), "{}", app.status);
    assert!(app.index.lock().unwrap().records.is_empty());
    // Yes, clicked: the map is begun, read and shown.
    press(&mut app, &["<m-g>"]);
    click_at(
        &mut app,
        app::Target::Key(crate::keymap::key_named("y").unwrap()),
    );
    assert!(!app.map_later);
    assert_eq!(app.map.mode, crate::map::Mode::Map);
    assert!(!app.index.lock().unwrap().records.is_empty());
    assert!(
        app.map
            .snapshot
            .as_ref()
            .is_some_and(|s| !s.dots.is_empty())
    );
}

// ---- regions of the map ----

/// The messages of the demo's map, by subject, and whether each is read.
pub(super) fn map_seen(app: &App) -> std::collections::BTreeMap<String, bool> {
    let s = app.map.snapshot.as_ref().unwrap();
    s.dots
        .iter()
        .map(|d| (d.subject.clone(), d.flags.seen))
        .collect()
}

#[test]
fn v_marks_the_cluster_and_u_marks_it_all_read() {
    let mut app = demo_map();
    let s = app.map.snapshot.clone().unwrap();
    // The cluster of the Grant's mail, with unread mail in it.
    let picked = s
        .dots
        .iter()
        .position(|d| d.subject.starts_with("R01"))
        .unwrap();
    app.map.cursor = Some(s.dots[picked].key.clone());
    press(&mut app, &["v"]);
    let region = app.map.region.clone().expect("marked");
    let cluster = s.group_of(picked).unwrap();
    assert_eq!(region.keys.len(), cluster.members.len());
    assert!(app.status.contains("marked"), "{}", app.status);
    // Said on its block.
    let screen = app::screen(&mut app, 150, 45);
    assert!(screen.contains("· all marked"), "{screen}");
    press(&mut app, &["u"]);
    let seen = map_seen(&app);
    for &i in &cluster.members {
        assert!(seen[&s.dots[i].subject], "{} read", s.dots[i].subject);
    }
    // On the server too.
    let r01 = app
        .shown
        .iter()
        .find(|m| m.subject.starts_with("R01"))
        .unwrap();
    assert!(r01.flags.seen);
    press(&mut app, &["u"]);
    assert!(
        !map_seen(&app)[&s.dots[picked].subject],
        "and all unread again"
    );
    press(&mut app, &["v"]);
    assert!(app.map.region.is_none());
}

#[test]
fn what_is_marked_is_trashed_or_archived_at_once() {
    let mut app = demo_map();
    let s = app.map.snapshot.clone().unwrap();
    let builds: Vec<crate::index::Key> = s
        .dots
        .iter()
        .filter(|d| d.address == "ci@runner.example")
        .map(|d| d.key.clone())
        .collect();
    assert_eq!(builds.len(), 3);
    app.map.region = Some(crate::map::Region { keys: builds });
    press(&mut app, &["d"]);
    assert!(app.map.region.is_none());
    // How it went, said once done: any key closes it.
    assert!(matches!(app.dialog, app::Dialog::Applied(_)));
    press(&mut app, &["x"]);
    let s = app.map.snapshot.clone().unwrap();
    assert!(
        s.dots
            .iter()
            .filter(|d| d.address == "ci@runner.example")
            .all(|d| !d.system.inbox),
        "{}",
        app.status
    );
    // Archived: out of the Inbox, into Archive.
    let hotel: Vec<crate::index::Key> = s
        .dots
        .iter()
        .filter(|d| d.address == "stay@hotel.example" || d.address == "travel@uni.example")
        .map(|d| d.key.clone())
        .collect();
    app.map.region = Some(crate::map::Region { keys: hotel });
    press(&mut app, &["e"]);
    let s = app.map.snapshot.clone().unwrap();
    for d in s
        .dots
        .iter()
        .filter(|d| d.address == "stay@hotel.example" || d.address == "travel@uni.example")
    {
        assert!(!d.system.inbox, "{}", d.subject);
        assert!(
            d.places.iter().any(|p| p.folder == "Archive"),
            "{}",
            d.subject
        );
    }
}

// ---- laying the map out again ----

#[test]
fn m_r_lays_the_map_out_again_to_keep_or_drop() {
    let mut app = demo_map();
    let kept = |app: &App| app.index.lock().unwrap().keep.layouts[&Lens::Both].clone();
    let before = kept(&app);
    press(&mut app, &["<m-r>"]);
    assert!(app.map.preview.is_some(), "{}", app.status);
    assert!(app.status.contains("esc drops it"), "{}", app.status);
    let screen = app::screen(&mut app, 150, 45);
    assert!(screen.contains("laid out again"), "{screen}");
    // Dropped, it is as it was.
    press(&mut app, &["<esc>"]);
    assert!(app.map.preview.is_none());
    assert_eq!(kept(&app), before);
    // Kept, it is the map from now on.
    press(&mut app, &["<m-r>", "<enter>"]);
    assert!(app.map.preview.is_none());
    assert!(app.status.contains("kept"), "{}", app.status);
    let after = kept(&app);
    assert_eq!(after.len(), before.len());
    assert_ne!(after, before);
    let s = app.map.snapshot.clone().unwrap();
    let at = s.at.as_ref().unwrap();
    let k = &s.model.keys[0];
    assert_eq!((at[0][0], at[0][1]), after[k]);
}

#[test]
fn m_r_is_kept_though_mail_comes_while_it_is_laid_out() {
    let mut app = demo_map();
    let s = app.map.snapshot.clone().unwrap();
    // Laid out from this snapshot, shown on a newer one with a message more.
    let at = crate::map::relayout(&s);
    let indexed = index::record::Indexed {
        uid: 999,
        size: 300,
        raw: raw(
            "Mira Okafor <mira.okafor@uni.example>",
            "Aims page, fourth pass",
            "",
            "Aim 2 is shorter now.\r\n",
        ),
        ..Default::default()
    };
    let r = index::Record::of("demo", "INBOX", 1, &indexed, 0);
    app.index
        .lock()
        .unwrap()
        .cache(vec![Cached::Upsert(Box::new(r))])
        .unwrap();
    app.settle(Duration::from_secs(5));
    let _ = app::screen(&mut app, 150, 45);
    let newer = app.map.snapshot.clone().unwrap();
    assert_eq!(newer.dots.len(), s.dots.len() + 1);
    let preview = crate::map::onto(&newer, &s.model.keys, &at).unwrap();
    assert_eq!(preview.len(), newer.dots.len());
    let k = &s.model.keys[0];
    assert_eq!(preview[newer.find(k).unwrap()], at[0]);
    // As the app takes it: shown, then kept.
    press(&mut app, &["<m-r>"]);
    assert!(app.map.preview.is_some(), "{}", app.status);
    press(&mut app, &["<enter>"]);
    assert!(app.status.contains("kept"), "{}", app.status);
}

#[test]
fn faithfulness_below_seventy_percent_suggests_m_r() {
    let mut app = demo_map();
    let faith = app.map.snapshot.as_ref().unwrap().faithfulness.unwrap();
    let screen = app::screen(&mut app, 160, 45);
    let said = format!("{:.0}% faithful", faith * 100.0);
    assert!(screen.contains(&said), "{said} in {screen}");
    assert_eq!(
        screen.contains("M-r lays it out again"),
        faith < 0.7,
        "{screen}"
    );
}

/// The layouts `index` keeps, as their bits, by lens.
pub(super) fn layout_bits(index: &Index) -> Vec<(Lens, Vec<u32>)> {
    index
        .keep
        .layouts
        .iter()
        .map(|(lens, at)| {
            let bits = at
                .values()
                .flat_map(|&(x, y)| [x.to_bits(), y.to_bits()])
                .collect();
            (*lens, bits)
        })
        .collect()
}

#[test]
fn a_rebuild_lays_out_the_same_map_from_the_same_mail_and_keeps_what_letterbox_knows() {
    let dir = tempfile::tempdir().unwrap();
    let id = identity();
    let mut once = demo_index();
    let said = crate::map::rebuild(&mut once).unwrap();
    assert!(
        said.contains(&format!("laid out {} messages", INBOX + ARCHIVE + 4 + 4)),
        "{said}"
    );
    assert_eq!(once.keep.layouts.len(), 3, "a layout each lens");
    let mut twice = demo_index();
    crate::map::rebuild(&mut twice).unwrap();
    assert_eq!(layout_bits(&once), layout_bits(&twice));

    // From the servers: what they said is let go and read again; a label
    // letterbox keeps, and a "not it", stay.
    {
        let mut index = Index::open_with(dir.path(), &id);
        let demo = Demo::new();
        let mut twin = demo.twin().unwrap();
        let shared = Mutex::new(index);
        sync_once(twin.as_mut(), &shared);
        index = shared.into_inner().unwrap();
        let key = index.records.keys().next().unwrap().clone();
        index
            .keep(vec![
                Kept::Label {
                    key: key.clone(),
                    label: "Lab".into(),
                    on: true,
                },
                Kept::NotIt {
                    key,
                    label: "Grant".into(),
                },
            ])
            .unwrap();
        index.forget_cache().unwrap();
        assert!(index.records.is_empty());
        let shared = Mutex::new(index);
        sync_once(twin.as_mut(), &shared);
        let mut index = shared.into_inner().unwrap();
        crate::map::rebuild(&mut index).unwrap();
        assert_eq!(index.records.len(), INBOX + ARCHIVE + 4 + 4);
    }
    let index = Index::open_with(dir.path(), &id);
    assert_eq!(index.records.len(), INBOX + ARCHIVE + 4 + 4);
    assert_eq!(index.keep.labels.len(), 1);
    assert_eq!(index.keep.not_it.len(), 1);
    assert_eq!(index.keep.layouts.len(), 3);
    assert!(
        index
            .keep
            .layouts
            .values()
            .all(|at| at.len() == INBOX + ARCHIVE + 4 + 4)
    );
}

// ---- the map's model on mail made up of known topics ----

/// Numbers that look random, for mail made up.
pub(super) struct Dice(u64);

impl Dice {
    pub(super) fn next(&mut self) -> u64 {
        self.0 = self.0.wrapping_add(0x9e37_79b9_7f4a_7c15);
        let mut z = self.0;
        z = (z ^ (z >> 30)).wrapping_mul(0xbf58_476d_1ce4_e5b9);
        z = (z ^ (z >> 27)).wrapping_mul(0x94d0_49bb_1331_11eb);
        z ^ (z >> 31)
    }

    pub(super) fn unit(&mut self) -> f64 {
        (self.next() >> 11) as f64 / (1u64 << 53) as f64
    }

    pub(super) fn below(&mut self, n: usize) -> usize {
        ((self.unit() * n as f64) as usize).min(n - 1)
    }

    /// One of `cumulative`'s places, as likely as its share of the last.
    pub(super) fn pick(&mut self, cumulative: &[f64]) -> usize {
        let at = self.unit() * cumulative[cumulative.len() - 1];
        cumulative
            .partition_point(|&c| c < at)
            .min(cumulative.len() - 1)
    }
}

/// A made-up word, the `i`th, of letters but `s`.
pub(super) fn made_word(mut i: usize) -> String {
    const LETTERS: &[u8] = b"abcdefghijklmnopqrtuvwxyz";
    let mut w = String::from("q");
    loop {
        w.push(LETTERS[i % LETTERS.len()] as char);
        i /= LETTERS.len();
        if i == 0 {
            return w;
        }
    }
}

pub(super) fn zipf(n: usize, shift: f64) -> Vec<f64> {
    let mut total = 0.0;
    (0..n)
        .map(|r| {
            total += 1.0 / (r as f64 + shift);
            total
        })
        .collect()
}

/// `n` messages made up of topics, as records, and each one's topic. A
/// topic has its people, some shared with others, a list now and then,
/// and words of its own; each message mixes a topic's words with words
/// any message might have, the commonest most often; a tenth are sent.
pub(super) fn made_mail(n: usize, seed: u64) -> (Vec<index::Record>, Vec<usize>) {
    let mut dice = Dice(seed);
    let words = 30_000;
    let any = zipf(words, 10.0);
    let topics = (n / 100).max(20);
    let people = topics * 2;
    let address = |p: usize| format!("p{p}@d{}.example", p % (people / 3 + 1));
    struct Topic {
        people: Vec<usize>,
        list: Option<String>,
        words: Vec<usize>,
    }
    let all: Vec<Topic> = (0..topics)
        .map(|t| {
            let list = (dice.unit() < 0.25).then(|| format!("list{t}.example"));
            let many = if list.is_some() { 1 } else { 1 + dice.below(5) };
            Topic {
                people: (0..many).map(|_| dice.below(people)).collect(),
                list,
                words: (0..30).map(|_| 200 + dice.below(words - 200)).collect(),
            }
        })
        .collect();
    let sizes = zipf(topics, 3.0);
    let mut records = Vec::with_capacity(n);
    let mut truth = Vec::with_capacity(n);
    for i in 0..n {
        let t = dice.pick(&sizes);
        let topic = &all[t];
        let word = |share: f64, dice: &mut Dice| {
            if dice.unit() < share {
                made_word(topic.words[dice.below(topic.words.len())])
            } else {
                made_word(dice.pick(&any))
            }
        };
        let subject: Vec<String> = (0..4).map(|_| word(0.6, &mut dice)).collect();
        let preview: Vec<String> = (0..20).map(|_| word(0.3, &mut dice)).collect();
        let person = address(topic.people[dice.below(topic.people.len())]);
        let sent = topic.list.is_none() && dice.unit() < 0.1;
        let mut to = vec!["you@home.example".to_string()];
        if dice.unit() < 0.3 {
            to.push(address(topic.people[dice.below(topic.people.len())]));
        }
        let (address, to) = if sent {
            ("you@home.example".to_string(), vec![person])
        } else {
            (person, to)
        };
        records.push(index::Record {
            key: index::Key {
                account: "made".into(),
                id: format!("id:{i}"),
            },
            from: address.clone(),
            address,
            to,
            list: topic.list.clone().unwrap_or_default(),
            subject: subject.join(" "),
            preview: preview.join(" "),
            date: Some(i as i64),
            ..Default::default()
        });
        truth.push(t);
    }
    (records, truth)
}

/// How the model does on made-up mail of `n` messages: how long each
/// step takes, and how many of each message's ten nearest share its topic.
pub(super) fn measure(n: usize) {
    use crate::model;
    use std::collections::BTreeMap;
    use std::time::Instant;
    let (records, truth) = made_mail(n, 7);
    let given: Vec<model::Given> = records
        .iter()
        .map(|r| (r, Vec::new(), r.address == "you@home.example"))
        .collect();
    let t = Instant::now();
    let m = model::Model::build(&given, &Default::default(), Lens::Both);
    let build = t.elapsed();
    let t = Instant::now();
    let c = model::cluster(&m, &[], 1);
    let clustering = t.elapsed();
    let t = Instant::now();
    let at = model::layout(&m, None, 1);
    let layout = t.elapsed();
    let f = model::faithfulness(&m, &at);
    let sample: Vec<usize> = (0..n).step_by((n / 500).max(1)).collect();
    let share = |near: &dyn Fn(usize) -> Vec<usize>| {
        let mut same = 0;
        let mut all = 0;
        for &i in &sample {
            for j in near(i) {
                same += usize::from(truth[j] == truth[i]);
                all += 1;
            }
        }
        same as f32 / all.max(1) as f32
    };
    // Of each message's ten nearest, how many share its topic: by tokens,
    // every pair compared; as found; by the vectors; on the map.
    let every: BTreeMap<usize, Vec<usize>> =
        model::nearest_by_every_pair(&given, Lens::Both, 10, sample.iter().copied())
            .into_iter()
            .map(|(i, near)| (i, near.into_iter().map(|x| x.0).collect()))
            .collect();
    let ceiling = share(&|i| every[&i].clone());
    let found = share(&|i| m.near[i].iter().take(10).map(|x| x.0).collect());
    let vectors = share(&|i| m.nearest(i, 10).into_iter().map(|x| x.0).collect());
    let on_map = share(&|i| {
        let mut all: Vec<(usize, f32)> = (0..n)
            .filter(|&j| j != i)
            .map(|j| {
                (
                    j,
                    (at[i][0] - at[j][0]).powi(2) + (at[i][1] - at[j][1]).powi(2),
                )
            })
            .collect();
        all.sort_by(|a, b| a.1.total_cmp(&b.1).then(a.0.cmp(&b.0)));
        all.into_iter().take(10).map(|x| x.0).collect()
    });
    // Clusters: how many messages are in the cluster most of their topic is.
    let mut by_topic: BTreeMap<usize, BTreeMap<usize, usize>> = BTreeMap::new();
    for (t, &of) in truth.iter().zip(&c.of) {
        *by_topic.entry(*t).or_default().entry(of).or_default() += 1;
    }
    let pure: usize = by_topic.values().map(|m| m.values().max().unwrap()).sum();
    eprintln!(
        "n {n:>6}: build {build:>6.0?} cluster {clustering:>6.0?} layout {layout:>6.0?} | \
         topic@10 every pair {ceiling:.2} found {found:.2} vectors {vectors:.2} map {on_map:.2} | \
         kept together {:.2} | faithful {:.0}%",
        pure as f32 / n as f32,
        100.0 * f
    );
}

#[test]
fn a_label_split_between_clusters_is_one_block_and_lights_all_its_mail() {
    let (mut records, truth) = made_mail(1_500, 11);
    // Work is six topics' label, far apart on the map.
    for (r, t) in records.iter_mut().zip(&truth) {
        if *t % 7 == 0 && *t < 42 {
            r.server = vec!["Work".into()];
        }
    }
    let work = records.iter().filter(|r| !r.server.is_empty()).count();
    let mut index = Index::memory();
    index
        .cache(
            records
                .into_iter()
                .map(|r| Cached::Upsert(Box::new(r)))
                .collect(),
        )
        .unwrap();
    let s = crate::map::compute(crate::map::input(&index, Lens::Both, None, None, true));
    let parts = s
        .clustering
        .clusters
        .iter()
        .filter(|c| c.label.as_deref() == Some("Work"))
        .count();
    assert!(parts > 1, "Work split between {parts} clusters");
    let nodes: Vec<&crate::map::Group> = s
        .groups
        .iter()
        .filter(|g| g.label.as_deref() == Some("Work"))
        .collect();
    assert_eq!(nodes.len(), 1, "one block for Work");
    let g = nodes[0];
    assert!(g.core.len() < g.members.len() && g.core.iter().all(|i| g.members.contains(i)));
    // Picked, it lights every message of Work, wherever it lies.
    let lit = (0..s.dots.len()).filter(|&i| s.picked_by(g.id, i)).count();
    assert!(lit >= work, "{lit} of {work}");
    assert!(
        (0..s.dots.len())
            .filter(|&i| !s.dots[i].labels.is_empty())
            .all(|i| s.picked_by(g.id, i))
    );
}

/// How long the map takes to follow the index, as the app has it do, on
/// `n` made-up messages: the first snapshot, then one after a new message
/// comes, and one after a message is read.
pub(super) fn measure_following(n: usize) {
    use std::time::Instant;
    let (records, _) = made_mail(n + 1, 7);
    let mut index = Index::memory();
    let mut records = records.into_iter();
    let last = records.next_back().unwrap();
    index
        .cache(records.map(|r| Cached::Upsert(Box::new(r))).collect())
        .unwrap();
    let step = |index: &Index, last: Option<&mut crate::map::Snapshot>| {
        let t = Instant::now();
        let (last, corpus) = match last {
            Some(s) => {
                let corpus = s.corpus.take();
                (Some(&*s), corpus)
            }
            None => (None, None),
        };
        let input = crate::map::input(index, Lens::Both, last, corpus, true);
        let taken = t.elapsed();
        let t = Instant::now();
        let s = crate::map::compute(input);
        (s, taken, t.elapsed())
    };
    let (mut first, taken, made) = step(&index, None);
    eprintln!("n {n:>6}: first: taken {taken:>6.0?} made {made:>6.0?}");
    // Kept, as the app keeps the layout made.
    let at = first.fresh.clone().unwrap();
    index
        .keep(vec![Kept::Layout {
            lens: Lens::Both,
            at,
        }])
        .unwrap();
    let (mut s, taken, made) = step(&index, Some(&mut first));
    eprintln!("          the layout kept: taken {taken:>6.0?} made {made:>6.0?}");
    let key = last.key.clone();
    index.cache(vec![Cached::Upsert(Box::new(last))]).unwrap();
    let (mut s2, taken, made) = step(&index, Some(&mut s));
    eprintln!("          a new message: taken {taken:>6.0?} made {made:>6.0?}");
    let flags = crate::mail::Flags {
        seen: true,
        ..Default::default()
    };
    index.cache(vec![Cached::Flags { key, flags }]).unwrap();
    let (_, taken, made) = step(&index, Some(&mut s2));
    eprintln!("          a message read: taken {taken:>6.0?} made {made:>6.0?}");
}

#[test]
#[ignore = "measures; run with --release --ignored --nocapture"]
fn how_long_the_map_takes_to_follow_the_index() {
    for n in [10_000, 50_000] {
        measure_following(n);
    }
}

#[test]
#[ignore = "measures; run with --release --ignored --nocapture"]
fn how_the_model_does_on_mail_of_known_topics() {
    for n in [2_000, 10_000, 50_000] {
        measure(n);
    }
}
