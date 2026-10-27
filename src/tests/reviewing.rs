//! The map for an agent, how well it would file, a message's pages, the
//! Inbox looked through, and what was asked of the servers, confirmed.

use super::*;

// ---- the map, for an agent ----

#[test]
fn an_agent_asks_what_is_near_a_message_where_it_would_go_and_whether_it_would_be_thrown_away() {
    use serde_json::json;
    let mut server = crate::mcp::Server::new(std::path::Path::new("/nowhere"), true).unwrap();
    // Listed as read only.
    let listed = server
        .answer(&json!({ "jsonrpc": "2.0", "id": 1, "method": "tools/list" }))
        .unwrap();
    for name in ["nearest_messages", "suggest_label", "would_throw_away"] {
        let tool = listed["result"]["tools"]
            .as_array()
            .unwrap()
            .iter()
            .find(|t| t["name"] == name)
            .unwrap_or_else(|| panic!("{name} listed"));
        assert_eq!(tool["annotations"]["readOnlyHint"], true, "{name}");
    }
    let page = call(&mut server, "list_messages", json!({ "limit": 50 }));
    let r01 = page["structuredContent"]["messages"]
        .as_array()
        .unwrap()
        .iter()
        .find(|m| m["subject"].as_str().unwrap_or_default().contains("R01"))
        .expect("the R01 letter in the Inbox")["uid"]
        .clone();
    // Near it: the grant's letters, nearest first.
    let near = call(
        &mut server,
        "nearest_messages",
        json!({ "uid": r01, "limit": 5 }),
    );
    assert_eq!(near["isError"], false, "{near}");
    let nearest = near["structuredContent"]["nearest"]
        .as_array()
        .unwrap()
        .clone();
    assert_eq!(nearest.len(), 5);
    let degrees: Vec<f64> = nearest
        .iter()
        .map(|m| m["degrees"].as_f64().unwrap())
        .collect();
    assert!(degrees.windows(2).all(|w| w[0] <= w[1]), "{degrees:?}");
    assert!(
        nearest
            .iter()
            .any(|m| m["labels"].as_array().unwrap().iter().any(|l| l == "Grant")),
        "{nearest:?}"
    );
    assert!(
        nearest
            .iter()
            .all(|m| m["uid"].is_u64() && m["folder"].is_string())
    );
    // Where it would go, and the labels nearest it.
    let sug = call(&mut server, "suggest_label", json!({ "uid": r01 }));
    let labels = sug["structuredContent"]["labels"]
        .as_array()
        .unwrap()
        .clone();
    assert!(!labels.is_empty() && labels[0]["degrees"].is_f64(), "{sug}");
    if let Some(go) = sug["structuredContent"]["would_go_to"].as_str() {
        assert_eq!(labels[0]["name"], go);
    }
    // Nothing taught yet: it says so, and throws nothing away.
    let thrown = call(&mut server, "would_throw_away", json!({ "uid": r01 }));
    let t = &thrown["structuredContent"];
    assert_eq!(t["ready"], false, "{thrown}");
    assert_eq!(t["would_throw_away"], false);
    assert!(t["odds"].is_f64());
    // One not there: why not.
    let none = call(&mut server, "nearest_messages", json!({ "uid": 999_999 }));
    assert_eq!(none["isError"], true);
}

#[test]
fn the_index_read_only_reads_all_of_it_and_writes_nothing() {
    let dir = tempfile::tempdir().unwrap();
    let id = identity();
    let (a, b) = (record("a", "Notes on the engine"), record("b", "Photo 51"));
    {
        let mut index = Index::open_with(dir.path(), &id);
        index
            .cache(vec![Cached::Upsert(Box::new(a.clone()))])
            .unwrap();
        index.teach(&a.key, false).unwrap();
        index.flush().unwrap();
    }
    let files = |d: &std::path::Path| {
        let mut f: Vec<_> = std::fs::read_dir(d)
            .unwrap()
            .map(|e| e.unwrap().file_name())
            .collect();
        f.sort();
        f
    };
    let before = files(dir.path());
    let mut read = Index::read_only(dir.path(), &id);
    assert!(read.records.contains_key(&a.key));
    assert_eq!(read.filter.kept, 1);
    // Changed in memory only.
    read.cache(vec![Cached::Upsert(Box::new(b.clone()))])
        .unwrap();
    read.flush().unwrap();
    drop(read);
    assert_eq!(files(dir.path()), before, "nothing written");
    assert!(
        !Index::open_with(dir.path(), &id)
            .records
            .contains_key(&b.key)
    );
}

// ---- how well the map would file what you filed ----

#[test]
fn the_map_is_graded_on_putting_each_filed_message_back_under_its_label() {
    // Grant letters and seminar notices, each filed under its own; a few
    // mislabelled, to be missed.
    let mut given: Vec<(index::Record, Vec<String>)> = Vec::new();
    for k in 0..12 {
        given.push((
            sent_by(
                &format!("g{k}"),
                "office@research.example",
                &format!("Grant budget aims {k}"),
            ),
            vec!["Grant".to_string()],
        ));
        given.push((
            sent_by(
                &format!("s{k}"),
                "talks@uni.example",
                &format!("Seminar speaker abstract {k}"),
            ),
            vec!["Seminars".to_string()],
        ));
    }
    let messages: Vec<crate::model::Given> =
        given.iter().map(|(r, l)| (r, l.clone(), false)).collect();
    let corpus = crate::model::Corpus::read(&messages, index::Lens::Both);
    let check = corpus.label_check(&messages, corpus.base());
    assert_eq!(check.total, 24);
    assert_eq!(check.hits, 24, "{check:?}");
    // Labels given at random: about half put back.
    let mixed: Vec<crate::model::Given> = given
        .iter()
        .enumerate()
        .map(|(k, (r, _))| {
            let l = if (k * 7) % 3 == 0 {
                "Grant"
            } else {
                "Seminars"
            };
            (r, vec![l.to_string()], false)
        })
        .collect();
    let corpus = crate::model::Corpus::read(&mixed, index::Lens::Both);
    let check = corpus.label_check(&mixed, corpus.base());
    assert!(check.hits < 20, "{check:?}");
}

#[test]
fn a_label_of_two_kinds_is_put_back_by_its_nearest_not_its_mean() {
    // Projects holds two kinds of mail, a grant's and a course's; Seminars
    // one kind, between them. By the mean, Projects' mail goes to Seminars;
    // by the nearest filed, to Projects.
    let mut given: Vec<(index::Record, Vec<String>)> = Vec::new();
    for k in 0..10 {
        given.push((
            sent_by(
                &format!("g{k}"),
                "office@research.example",
                &format!("Grant budget aims {k}"),
            ),
            vec!["Projects".to_string()],
        ));
        given.push((
            sent_by(
                &format!("c{k}"),
                "teach@uni.example",
                &format!("Course syllabus marking {k}"),
            ),
            vec!["Projects".to_string()],
        ));
        given.push((
            sent_by(
                &format!("s{k}"),
                "talks@uni.example",
                &format!("Seminar speaker abstract {k}"),
            ),
            vec!["Seminars".to_string()],
        ));
    }
    let messages: Vec<crate::model::Given> =
        given.iter().map(|(r, l)| (r, l.clone(), false)).collect();
    let corpus = crate::model::Corpus::read(&messages, index::Lens::Both);
    let near = corpus.label_check_nearest(&messages, corpus.base(), crate::model::VOTERS, 90.0);
    assert_eq!((near.hits, near.total), (30, 30), "{near:?}");
    // Always the commonest label: the share of it.
    let common = crate::model::commonest_label_check(&messages);
    assert_eq!((common.hits, common.total), (20, 30));
}

#[test]
fn where_mail_would_go_is_voted_by_the_filed_mail_nearest_it() {
    let mut given: Vec<(index::Record, Vec<String>)> = Vec::new();
    for k in 0..10 {
        given.push((
            sent_by(
                &format!("g{k}"),
                "office@research.example",
                &format!("Grant budget aims {k}"),
            ),
            vec!["Projects".to_string()],
        ));
        given.push((
            sent_by(
                &format!("c{k}"),
                "teach@uni.example",
                &format!("Course syllabus marking {k}"),
            ),
            vec!["Projects".to_string()],
        ));
        given.push((
            sent_by(
                &format!("s{k}"),
                "talks@uni.example",
                &format!("Seminar speaker abstract {k}"),
            ),
            vec!["Seminars".to_string()],
        ));
    }
    let course = sent_by(
        "new",
        "teach@uni.example",
        "Course syllabus marking, the last",
    );
    let talk = sent_by(
        "talk",
        "talks@uni.example",
        "Seminar speaker abstract, the next",
    );
    given.push((course.clone(), Vec::new()));
    given.push((talk.clone(), Vec::new()));
    let messages: Vec<crate::model::Given> =
        given.iter().map(|(r, l)| (r, l.clone(), false)).collect();
    let model = crate::model::Model::build(&messages, &Default::default(), index::Lens::Both);
    let at = |r: &index::Record| model.place[&r.key];
    let voted = model.voted(|_, _| false);
    // A course's mail, though Projects' mean is far from it: Projects.
    assert_eq!(voted[at(&course)].as_deref(), Some("Projects"));
    assert_eq!(voted[at(&talk)].as_deref(), Some("Seminars"));
    // Filed mail is voted for nothing; a label turned down is not voted.
    assert!(voted[0].is_none());
    let voted = model.voted(|i, l| i == at(&course) && l == "projects");
    assert_ne!(voted[at(&course)].as_deref(), Some("Projects"));
}

// ---- a message's pages, in the terminal ----

#[cfg(not(feature = "pages"))]
#[test]
fn v_built_without_pages_says_how_to_have_them() {
    let mut app = demo();
    press(&mut app, &["<enter>", "V"]);
    assert!(app.status.contains("--features pages"), "{}", app.status);
}

#[cfg(feature = "pages")]
#[test]
fn v_shows_the_message_read_as_its_pages_and_v_again_its_text() {
    let mut app = demo();
    let noether = app
        .shown
        .iter()
        .position(|l| l.summary.subject.contains("Symmetry"))
        .expect("Noether's letter, in HTML");
    app.list.select(Some(noether));
    press(&mut app, &["<enter>"]);
    app.settle(Duration::from_secs(5));
    // Not asked of the terminal as letterbox starts: only once V is.
    assert!(app.picker.is_none(), "not asked yet");
    // A terminal that draws no images: said, and O offered.
    app.picker = Some(None);
    press(&mut app, &["V"]);
    assert!(app.paged.is_none());
    assert!(app.status.contains("O"), "{}", app.status);
    // One that does: typeset, and shown a page at a time.
    app.picker = Some(Some(ratatui_image::picker::Picker::halfblocks()));
    press(&mut app, &["V"]);
    app.settle(Duration::from_secs(60));
    let paged = app.paged.as_ref().expect("its pages");
    assert!(!paged.is_empty());
    let screen = app::screen(&mut app, 120, 40);
    assert!(screen.contains("page 1 of"), "{screen}");
    // V again: its text.
    press(&mut app, &["V"]);
    assert!(app.paged.is_none());
    let screen = app::screen(&mut app, 120, 40);
    assert!(screen.contains("differentiable symmetry"), "{screen}");
}

#[cfg(feature = "pages")]
#[test]
fn pages_turn_as_far_as_there_are() {
    let blank = || image::RgbaImage::new(10, 14);
    let mut paged =
        crate::pages::Paged::new((0, "INBOX".to_string(), 1), vec![blank(), blank(), blank()]);
    paged.step(1);
    paged.step(1);
    paged.step(1);
    assert_eq!(paged.at, 2);
    paged.step(-5);
    assert_eq!(paged.at, 0);
    let picker = ratatui_image::picker::Picker::halfblocks();
    assert!(paged.drawn(&picker).is_some());
    // Turned on and back, each page still there to draw.
    paged.step(1);
    assert!(paged.drawn(&picker).is_some());
    paged.step(-1);
    assert!(paged.drawn(&picker).is_some());
}

#[cfg(feature = "pages")]
#[test]
fn pages_are_made_to_fit_the_pane_whole() {
    let tall = image::RgbaImage::new(100, 200);
    let wide = image::RgbaImage::new(100, 50);
    let fitted = crate::pages::fit(vec![tall, wide], 100, 100);
    assert_eq!(fitted[0].dimensions(), (50, 100));
    assert_eq!(fitted[1].dimensions(), (100, 50));
}

/// The demo's Noether letter read, in HTML, shown as its pages.
#[cfg(feature = "pages")]
pub(super) fn noether_paged() -> App {
    let mut app = demo();
    let noether = app
        .shown
        .iter()
        .position(|l| l.summary.subject.contains("Symmetry"))
        .expect("Noether's letter, in HTML");
    app.list.select(Some(noether));
    press(&mut app, &["<enter>"]);
    app.settle(Duration::from_secs(5));
    app.picker = Some(Some(ratatui_image::picker::Picker::halfblocks()));
    press(&mut app, &["V"]);
    app.settle(Duration::from_secs(60));
    assert!(app.paged.is_some(), "{}", app.status);
    app
}

#[cfg(feature = "pages")]
#[test]
fn pages_are_of_the_message_read_and_let_go_once_another_is() {
    let mut app = noether_paged();
    // Of the same number in another folder: not its pages.
    app.paged.as_mut().unwrap().place.1 = "Archive".to_string();
    let screen = app::screen(&mut app, 120, 40);
    assert!(!screen.contains("page 1 of"), "{screen}");
    assert!(app.paged.is_none(), "let go");
    // Another message read: its pages let go, and V shows the new one's
    // at once.
    let mut app = noether_paged();
    let uid = app.reading.as_ref().unwrap().uid;
    press(&mut app, &["n"]);
    app.settle(Duration::from_secs(5));
    assert_ne!(
        app.reading.as_ref().map(|r| r.uid),
        Some(uid),
        "n reads the next"
    );
    let _ = app::screen(&mut app, 120, 40);
    assert!(app.paged.is_none());
    press(&mut app, &["V"]);
    assert!(!app.status.starts_with("its text"), "{}", app.status);
    app.settle(Duration::from_secs(60));
    assert_eq!(
        app.paged.as_ref().map(|p| p.place.clone()),
        app.read_place(),
        "{}",
        app.status
    );
}

#[cfg(feature = "pages")]
#[test]
fn v_while_its_pages_are_typeset_lets_them_go() {
    let mut app = demo();
    let noether = app
        .shown
        .iter()
        .position(|l| l.summary.subject.contains("Symmetry"))
        .unwrap();
    app.list.select(Some(noether));
    press(&mut app, &["<enter>"]);
    app.settle(Duration::from_secs(5));
    app.picker = Some(Some(ratatui_image::picker::Picker::halfblocks()));
    press(&mut app, &["V", "V"]);
    assert!(app.status.contains("its text"), "{}", app.status);
    app.settle(Duration::from_secs(60));
    assert!(app.paged.is_none(), "shown after all");
    assert!(!app.status.contains("pages"), "{}", app.status);
}

#[cfg(feature = "pages")]
#[test]
fn only_a_terminal_that_says_it_draws_images_is_asked() {
    let env = |pairs: &'static [(&'static str, &'static str)]| {
        move |k: &str| {
            pairs
                .iter()
                .find(|(n, _)| *n == k)
                .map(|(_, v)| v.to_string())
        }
    };
    for says in [
        env(&[("KITTY_WINDOW_ID", "1")]),
        env(&[("TERM", "xterm-kitty")]),
        env(&[("TERM_PROGRAM", "WezTerm")]),
        env(&[("TERM_PROGRAM", "ghostty")]),
        env(&[("TERM_PROGRAM", "iTerm.app")]),
    ] {
        assert!(crate::pages::draws_images(says));
    }
    // Terminal, and one that says nothing: never asked, never kept waiting.
    assert!(!crate::pages::draws_images(env(&[(
        "TERM_PROGRAM",
        "Apple_Terminal"
    )])));
    assert!(!crate::pages::draws_images(env(&[(
        "TERM",
        "xterm-256color"
    )])));
    assert!(!crate::pages::draws_images(env(&[])));
    // Within tmux or screen, which pass no images on, though the terminal
    // outside them would draw them.
    assert!(!crate::pages::draws_images(env(&[
        ("KITTY_WINDOW_ID", "1"),
        ("TMUX", "/tmp/tmux-501/default,1,0")
    ])));
    assert!(!crate::pages::draws_images(env(&[
        ("WEZTERM_EXECUTABLE", "/Applications/WezTerm.app"),
        ("STY", "1.ttys001.host")
    ])));
    assert!(!crate::pages::draws_images(env(&[
        ("GHOSTTY_RESOURCES_DIR", "/x"),
        ("TERM", "screen-256color")
    ])));
}

// ---- as reviewed ----

#[test]
fn a_late_answer_of_paperstack_never_lands_on_another_list_of_papers() {
    let mut app = reading_the_alert();
    let shelf = Arc::new(Mutex::new(Shelf::default()));
    *app.library.lock().unwrap() = Some(Box::new(Shared(shelf.clone())));
    // paperstack slow: P asked, let go, and asked again.
    let held = shelf.lock().unwrap();
    press(&mut app, &["P", "<esc>", "P"]);
    // As if another message's, of fewer papers, were listed now.
    if let app::Dialog::Papers(p) = &mut app.dialog {
        p.papers.truncate(1);
        p.standing.truncate(1);
    }
    drop(held);
    app.settle(Duration::from_secs(5));
    let _ = app::screen(&mut app, 120, 40);
    let app::Dialog::Papers(p) = &app.dialog else {
        panic!("closed: {}", app.status);
    };
    assert_eq!(p.standing.len(), p.papers.len());
    press(&mut app, &["<enter>"]);
    let _ = app::screen(&mut app, 120, 40);
}

#[test]
fn an_ampersand_before_a_letter_of_more_than_a_byte_is_read() {
    assert_eq!(
        crate::papers::decode("Smith & Müller — Nature &amp; more"),
        "Smith & Müller — Nature & more"
    );
}

#[test]
fn going_to_the_list_lets_go_of_what_was_moving_and_marked_on_the_map() {
    let mut app = demo_map();
    let s = app.map.snapshot.clone().unwrap();
    app.map.region = Some(crate::map::Region {
        keys: vec![s.dots[0].key.clone()],
    });
    app.map.moving = Some(vec![s.dots[1].key.clone()]);
    press(&mut app, &["<m-l>"]);
    assert!(app.map.moving.is_none());
    assert!(app.map.region.is_none(), "marked on the map, let go");
    // Marked in the list, kept as the list is asked for again.
    press(&mut app, &["v"]);
    assert!(app.map.region.is_some(), "{}", app.status);
    press(&mut app, &["<m-l>"]);
    assert!(app.map.region.is_some(), "marked in the list, kept");
}

#[test]
fn what_is_marked_in_the_list_is_let_go_as_another_folder_is_opened() {
    let mut app = demo();
    press(&mut app, &["v"]);
    assert!(app.map.region.is_some(), "{}", app.status);
    app.focus = app::Focus::Side;
    app.row = app
        .rows
        .iter()
        .position(|r| match *r {
            app::Row::Folder(a, i) => app.accounts[a].folders[i].name == "Archive",
            _ => false,
        })
        .unwrap();
    press(&mut app, &["<enter>"]);
    app.settle(Duration::from_secs(5));
    assert!(app.map.region.is_none(), "marked in the Inbox, let go");
}

#[test]
fn on_the_map_m_on_the_message_read_drops_down_nothing_unseen() {
    let mut app = demo_map();
    app.focus = app::Focus::Reader;
    press(&mut app, &["m"]);
    assert!(app.menu.is_none(), "a menu drawn nowhere");
}

#[test]
fn mail_read_is_never_thrown_away_on_its_own() {
    let mut app = demo_taught();
    let sure: Vec<crate::index::Key> = {
        let index = app.index.lock().unwrap();
        let known = index.known();
        index
            .records
            .values()
            .filter(|r| index.system(&r.key).inbox)
            .filter(|r| {
                index.filter.odds(&crate::filter::Filter::tokens(r, &known)) >= crate::filter::AUTO
            })
            .map(|r| r.key.clone())
            .collect()
    };
    assert!(!sure.is_empty());
    // Read, all of them, and left in the Inbox.
    for key in &sure {
        app.index
            .lock()
            .unwrap()
            .records
            .get_mut(key)
            .unwrap()
            .flags
            .seen = true;
    }
    app.auto = crate::filter::Auto {
        on: true,
        since: 0,
        debriefed: 0,
    };
    app.throw_away_on_its_own();
    app.settle(Duration::from_secs(5));
    let auto = app
        .index
        .lock()
        .unwrap()
        .filter
        .taught
        .iter()
        .filter(|t| t.auto)
        .count();
    assert_eq!(auto, 0);
}

/// A library that cannot be reached, or that says no.
pub(super) struct Failing(&'static str);

impl crate::paperstack::Library for Failing {
    fn search(&mut self, _: &str, _: usize) -> anyhow::Result<Vec<crate::paperstack::Found>> {
        match self.0 {
            "pipe" => Err(std::io::Error::from(std::io::ErrorKind::BrokenPipe).into()),
            said => Err(anyhow::anyhow!("paperstack: {said}")),
        }
    }
    fn add(&mut self, _: &str) -> anyhow::Result<crate::paperstack::Found> {
        anyhow::bail!("paperstack: no")
    }
    fn star(&mut self, _: &str) -> anyhow::Result<()> {
        anyhow::bail!("paperstack: no")
    }
}

#[test]
fn paperstack_gone_is_run_again_and_paperstack_saying_no_is_kept() {
    let mut app = reading_the_alert();
    *app.library.lock().unwrap() = Some(Box::new(Failing("no such paper")));
    press(&mut app, &["P"]);
    app.settle(Duration::from_secs(5));
    assert!(app.library.lock().unwrap().is_some(), "it answered");
    press(&mut app, &["<esc>"]);
    *app.library.lock().unwrap() = Some(Box::new(Failing("pipe")));
    press(&mut app, &["P"]);
    app.settle(Duration::from_secs(5));
    assert!(app.library.lock().unwrap().is_none(), "run again next time");
}

#[test]
fn in_the_list_all_marked_are_picked_where_to_go_on_the_map() {
    let mut app = demo();
    press(&mut app, &["v", "v"]);
    let marked = app.map.region.as_ref().unwrap().keys.clone();
    assert_eq!(marked.len(), 2);
    press(&mut app, &["k", "m"]);
    let on_map = app
        .menu_items()
        .iter()
        .position(|p| matches!(p, app::Pick::OnMap))
        .unwrap();
    for _ in 0..on_map {
        press(&mut app, &["<down>"]);
    }
    press(&mut app, &["<enter>"]);
    let mut moving = app.map.moving.clone().expect("moving");
    moving.sort();
    let mut marked = marked;
    marked.sort();
    assert_eq!(moving, marked);
    assert!(app.map.region.is_none());
}

#[test]
fn an_image_whose_name_begins_another_s_leaves_it_whole() {
    let html = r#"<img src="cid:1"><img src="cid:10">"#;
    let images = vec![
        ("1".to_string(), "image/png".to_string(), b"one".to_vec()),
        ("10".to_string(), "image/png".to_string(), b"ten".to_vec()),
    ];
    let page = crate::app::inline_images(html, &images);
    assert!(page.contains("data:image/png;base64,b25l\""), "{page}");
    assert!(page.contains("data:image/png;base64,dGVu\""), "{page}");
}

#[test]
fn one_letterbox_writes_the_index_at_a_time() {
    let dir = tempfile::tempdir().unwrap();
    let id = identity();
    let first = Index::open_with(dir.path(), &id);
    assert!(first.writes());
    // Another, as `filter learn` while letterbox is open: read, never
    // written, and why said.
    let second = Index::open_with(dir.path(), &id);
    assert!(!second.writes());
    assert!(
        second.fault.as_deref().is_some_and(|f| f.contains("open")),
        "{:?}",
        second.fault
    );
    drop(second);
    drop(first);
    assert!(Index::open_with(dir.path(), &id).writes());
}

// ---- What was asked of the servers, confirmed ----

pub(super) fn job(uid: u32, goal: crate::confirm::Goal) -> crate::confirm::Job {
    crate::confirm::Job {
        account: 0,
        folder: "INBOX".to_string(),
        uid,
        key: None,
        subject: format!("message {uid}"),
        goal,
        choice: None,
        state: crate::confirm::State::Asked,
    }
}

#[test]
fn what_is_asked_of_a_server_is_confirmed_once_the_mail_is_seen_so() {
    use crate::confirm::{Batch, Goal, State};
    let mut batch = Batch::default();
    batch.ask(job(1, Goal::Trashed));
    batch.ask(job(2, Goal::Filed("Grant".into())));
    batch.ask(job(3, Goal::Archived));
    assert!(batch.busy());
    assert_eq!(batch.said(), "applying: 0 of 3 confirmed");
    // Answered: done, failed.
    assert!(batch.answered(0, "INBOX", 1, Ok(())));
    assert!(batch.answered(0, "INBOX", 2, Ok(())));
    assert!(batch.answered(0, "INBOX", 3, Err("no such label".into())));
    assert!(!batch.answered(0, "INBOX", 9, Ok(())), "not one asked");
    assert_eq!(batch.jobs[2].state, State::Failed("no such label".into()));
    // A look at the mail: 1 has left, 2 not yet; another, 2 has.
    batch.looked(0, |j| Some(j.uid == 1));
    assert_eq!(batch.jobs[0].state, State::Confirmed);
    assert!(matches!(batch.jobs[1].state, State::Done { .. }));
    assert!(batch.busy());
    assert!(batch.settled().is_none(), "not till all are");
    batch.looked(0, |_| Some(true));
    assert!(!batch.busy());
    assert_eq!(batch.said(), "2 of 3 confirmed · 1 failed");
    // Settled, said once.
    assert!(batch.settled().is_some());
    assert!(batch.settled().is_none());
    // One the mail still has, look after look: failed.
    batch.ask(job(4, Goal::Trashed));
    assert_eq!(batch.jobs.len(), 1, "a new batch, the last settled");
    batch.answered(0, "INBOX", 4, Ok(()));
    for _ in 0..crate::confirm::LOOKS {
        batch.looked(0, |_| Some(false));
    }
    assert!(matches!(batch.jobs[0].state, State::Failed(_)));
    // Where nothing looks at the mail, its server's word is taken.
    let mut batch = Batch::default();
    batch.ask(job(5, Goal::Archived));
    batch.answered(0, "INBOX", 5, Ok(()));
    batch.trust(0);
    assert_eq!(batch.jobs[0].state, State::Confirmed);
}

/// The demo, its server saying no to moving the message with `subject`.
pub(super) fn demo_refusing(subject: &str) -> App {
    let store = Demo::new();
    store.refuse(subject);
    let mut app = App::new(
        vec![("Demo".to_string(), Box::new(store) as Box<dyn Store>)],
        PathBuf::from("/nowhere/accounts.toml"),
    );
    app.ledger = scratch_ledger();
    app.settle(Duration::from_secs(5));
    app
}

#[test]
fn what_the_inbox_looked_through_does_is_confirmed_and_what_failed_is_said() {
    let refused = "R01 resubmission: internal deadline 3 November";
    let mut app = demo_refusing(refused);
    press(&mut app, &["<m-g>", "i"]);
    let s = app.map.snapshot.clone().unwrap();
    let rows = app.review_rows();
    let r01 = rows
        .iter()
        .copied()
        .find(|&i| s.dots[i].subject == refused)
        .unwrap();
    let other = rows.iter().copied().find(|&i| i != r01).unwrap();
    let other_subject = s.dots[other].subject.clone();
    for i in [r01, other] {
        app.review
            .as_mut()
            .unwrap()
            .chosen
            .insert(s.dots[i].key.clone(), app::Choice::Discard);
    }
    press(&mut app, &["$", "y"]);
    app.settle(Duration::from_secs(5));
    // A pop-up says how it went: the one done, the one not, and why.
    let screen = app::screen(&mut app, 150, 45);
    assert!(screen.contains("1 of 2 confirmed · 1 failed"), "{screen}");
    assert!(screen.contains("R01 resubmission"), "{screen}");
    assert!(screen.contains("the server said no"), "{screen}");
    // Any key closes it. The one that failed is looked through still, as
    // it was chosen; the other gone.
    press(&mut app, &["x"]);
    assert!(!app.dialog.is_open());
    let s = app.map.snapshot.clone().unwrap();
    let r01 = s.dots.iter().position(|d| d.subject == refused).unwrap();
    assert!(app.review_rows().contains(&r01));
    assert_eq!(app.choice(r01), app::Choice::Discard);
    let screen = app::screen(&mut app, 170, 45);
    assert!(screen.contains("✗ Research Office"), "{screen}");
    assert!(
        !app.review_rows()
            .iter()
            .any(|&i| s.dots[i].subject == other_subject)
    );
}

#[test]
fn what_is_done_in_the_list_is_confirmed_by_a_pop_up() {
    let mut app = demo();
    press(&mut app, &[" ", " ", "d"]);
    app.settle(Duration::from_secs(5));
    let screen = app::screen(&mut app, 150, 45);
    assert!(screen.contains("2 of 2 confirmed"), "{screen}");
    press(&mut app, &["x"]);
    assert!(!app.dialog.is_open());
}

#[test]
fn what_was_chosen_is_not_done_again_while_the_last_is_waited_for() {
    let mut app = demo_map();
    press(&mut app, &["i", "d"]);
    app.batch.ask(job(1, crate::confirm::Goal::Trashed));
    press(&mut app, &["$"]);
    assert!(!app.dialog.is_open());
    assert!(app.status.contains("still applying"), "{}", app.status);
    // How far it has come, at the foot, over anything else said.
    let screen = app::screen(&mut app, 170, 40);
    assert!(screen.contains("applying: 0 of 1 confirmed"), "{screen}");
}
