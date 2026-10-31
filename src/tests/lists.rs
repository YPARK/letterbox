//! The list and the map one key apart, papers into paperstack, marking in
//! the list, and threads.

use super::*;

// ---- the list and the map, one key apart ----

#[test]
fn m_goes_between_the_map_and_the_list_and_lets_go_of_what_was_the_maps() {
    let mut app = demo();
    press(&mut app, &["M"]);
    assert_eq!(app.map.mode, crate::map::Mode::Map);
    // Nothing is marked on the map; back in the list, nothing moving.
    press(&mut app, &["v"]);
    assert!(app.map.region.is_none());
    press(&mut app, &["M"]);
    assert_eq!(app.map.mode, crate::map::Mode::List);
    assert!(app.map.region.is_none() && app.map.moving.is_none());
    // The view menu open, M closes it and goes to the list.
    press(&mut app, &["M", ",", "f", "M"]);
    assert!(!app.dialog.is_open());
    assert_eq!(app.map.mode, crate::map::Mode::List);
    // From the Inbox looked through, the list is picked, not the message.
    press(&mut app, &["M", "i", "M"]);
    assert_eq!(app.focus, app::Focus::List);
    // A cluster listed counts what it lists.
    press(&mut app, &["M", "Z", "<enter>", "M"]);
    let screen = app::screen(&mut app, 150, 30);
    assert!(!screen.contains(" 0 messages"), "{screen}");
}

#[test]
fn m_drops_down_the_labels_over_a_message_read_alone() {
    let mut app = demo();
    // Narrow: the message read has the screen, no row of it in sight.
    app::screen(&mut app, 70, 30);
    press(&mut app, &["<enter>"]);
    assert_eq!(app.focus, app::Focus::Reader);
    press(&mut app, &["m"]);
    assert!(app.menu.is_some());
    let screen = app::screen(&mut app, 70, 30);
    assert!(
        screen.contains("Move to") && screen.contains("pick on the map"),
        "{screen}"
    );
}

#[test]
fn a_flag_toggled_with_the_list_out_of_sight_is_said() {
    let mut app = demo();
    app::screen(&mut app, 70, 30);
    press(&mut app, &["<enter>", "!"]);
    assert_eq!(app.status, "marked flagged");
    press(&mut app, &["!"]);
    assert_eq!(app.status, "marked unflagged");
    press(&mut app, &["u"]);
    assert_eq!(app.status, "marked unread");
}

#[test]
fn in_the_list_a_message_is_filed_as_in_the_map() {
    let mut app = demo();
    let subject = app.current().unwrap().summary.subject.clone();
    // m drops down the labels; ⏎ files it there at once.
    press(&mut app, &["m"]);
    assert!(app.menu.is_some());
    let screen = app::screen(&mut app, 150, 30);
    assert!(screen.contains("Move to"), "{screen}");
    let first = match &app.menu_items()[0] {
        app::Pick::Label(n, _) => n.clone(),
        other => panic!("{other:?}"),
    };
    press(&mut app, &["<enter>"]);
    assert!(app.menu.is_none());
    app.settle(Duration::from_secs(5));
    let (labels, inbox) = dot_labels(&app, &subject);
    assert!(labels.contains(&first) && !inbox, "{labels:?}");
    assert!(
        !app.shown.iter().any(|l| l.summary.subject == subject),
        "out of the Inbox listed"
    );
    // N names a new label for the next; esc lets it go.
    let next = app.current().unwrap().summary.subject.clone();
    press(&mut app, &["N"]);
    assert!(matches!(app.dialog, app::Dialog::Label(_)));
    press(&mut app, &["<esc>"]);
    assert!(app.map.moving.is_none());
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
    app.settle(Duration::from_secs(5));
    let (labels, inbox) = dot_labels(&app, &next);
    assert!(
        labels.contains(&"Engines".to_string()) && !inbox,
        "{labels:?}"
    );
    // y files one where it would go, as the map would.
    let s = app.map.snapshot.clone().unwrap();
    let going = app
        .shown
        .iter()
        .position(|l| {
            s.dots.iter().enumerate().any(|(i, d)| {
                d.subject == l.summary.subject && d.system.inbox && s.suggestion(i).is_some()
            })
        })
        .expect("one that would go somewhere");
    let subject = app.shown[going].summary.subject.clone();
    let i = s.dots.iter().position(|d| d.subject == subject).unwrap();
    let label = s.suggestion(i).unwrap().to_string();
    app.list.select(Some(going));
    press(&mut app, &["y"]);
    app.settle(Duration::from_secs(5));
    let (labels, inbox) = dot_labels(&app, &subject);
    assert!(labels.contains(&label) && !inbox, "{labels:?}");
}

// ---- papers in mail, into paperstack ----

pub(super) const SCHOLAR: &str = r#"<html><body>
<h3 style="font-weight:normal"><a href="https://scholar.google.com/scholar_url?url=https://www.nature.com/articles/s41586-026-11078-2&amp;hl=en&amp;sa=X&amp;oi=scholaralrt" class="gse_alrt_title">Population-scale immune <b>multiome</b> atlas reveals regulatory disease mechanisms</a></h3>
<div style="color:#006621">M Kanai, TM Delorey, J Honkanen - Nature, 2026</div>
<div class="gse_alrt_sni">The immune system is shaped by genetics &hellip;</div>
<h3><a href="https://scholar.google.com/scholar_url?url=https://arxiv.org/abs/2410.01234&amp;hl=en&amp;oi=scholaralrt" class="gse_alrt_title">Learning gene programs from single-cell data with transformers</a></h3>
<div style="color:#006621">A Author, B Author - arXiv preprint, 2026</div>
<p><a href="https://scholar.google.com/scholar_alerts?view_op=cancel_alert_options&amp;x=1">Cancel alert</a>
<a href="https://scholar.google.com/scholar_alerts?view_op=list_alerts">List alerts</a></p>
</body></html>"#;

pub(super) const NEJM: &str = r#"<table><tr><td>
<a href="https://click.nejm.org/?qs=abc123"><span>Large Language Models for Clinical Trial Matching in Oncology</span></a><br><span>J. Smith and Others</span>
</td></tr><tr><td>
<a href="https://ai.nejm.org/doi/full/10.1056/AIoa2500123?query=TOC">A Benchmark of Medical Reasoning in Vision&#8211;Language Models</a>
</td></tr><tr><td>
<a href="https://click.nejm.org/?qs=def">View Table of Contents</a>
<a href="https://click.nejm.org/?qs=x">Unsubscribe from these emails</a>
</td></tr></table>"#;

#[test]
fn the_papers_of_a_scholar_alert_are_found_with_where_they_are() {
    let found = crate::papers::in_message("scholaralerts-noreply@google.com", Some(SCHOLAR), "");
    let titles: Vec<&str> = found.iter().map(|p| p.title.as_str()).collect();
    assert_eq!(
        titles,
        [
            "Population-scale immune multiome atlas reveals regulatory disease mechanisms",
            "Learning gene programs from single-cell data with transformers"
        ]
    );
    // The address Scholar sends it on to, and who wrote it where.
    assert_eq!(
        found[0].what,
        "https://www.nature.com/articles/s41586-026-11078-2"
    );
    assert_eq!(
        found[0].about,
        "M Kanai, TM Delorey, J Honkanen - Nature, 2026"
    );
    assert_eq!(found[1].what, "arXiv:2410.01234");
}

#[test]
fn a_journals_table_of_contents_gives_its_papers_by_doi_or_title() {
    let found = crate::papers::in_message("nejmai@nejm.org", Some(NEJM), "");
    let titles: Vec<&str> = found.iter().map(|p| p.title.as_str()).collect();
    assert_eq!(
        titles,
        [
            "Large Language Models for Clinical Trial Matching in Oncology",
            "A Benchmark of Medical Reasoning in Vision–Language Models"
        ]
    );
    // Behind a link that tells nothing, by its title; else by its DOI.
    assert_eq!(found[0].what, found[0].title);
    assert_eq!(found[1].what, "10.1056/AIoa2500123");
    // Mail from no journal: only links that say they are papers.
    let found = crate::papers::in_message("friend@example.com", Some(NEJM), "");
    assert_eq!(found.len(), 1);
    assert_eq!(found[0].what, "10.1056/AIoa2500123");
}

#[test]
fn an_arxiv_listing_in_text_gives_its_papers() {
    let text = "------\narXiv:2410.05678\nDate: Mon, 21 Oct 2026\nTitle: Sparse autoencoders for gene\n  regulatory networks\nAuthors: C. Author\n------\n";
    let found = crate::papers::in_message("no-reply@arxiv.org", None, text);
    assert_eq!(found.len(), 1);
    assert_eq!(found[0].what, "arXiv:2410.05678");
    assert_eq!(
        found[0].title,
        "Sparse autoencoders for gene regulatory networks"
    );
}

pub(super) use crate::demo::Shelf;

#[test]
fn a_paper_is_in_your_library_like_what_you_starred_or_new() {
    use crate::paperstack::{Found, Standing, standing};
    let mut shelf = Shelf {
        papers: vec![
            Found {
                key: "Kanai2026".into(),
                title:
                    "Population-scale immune multiome atlas reveals regulatory disease mechanisms"
                        .into(),
                starred: true,
            },
            Found {
                key: "Theodoris2023".into(),
                title: "Transfer learning enables predictions in network biology".into(),
                starred: true,
            },
        ],
        ..Default::default()
    };
    let paper = |title: &str| crate::papers::Paper {
        title: title.into(),
        what: title.into(),
        about: String::new(),
        link: None,
    };
    let have = standing(
        &mut shelf,
        &paper("Population-scale immune multiome atlas reveals regulatory disease mechanisms"),
    )
    .unwrap();
    assert_eq!(
        have,
        Standing::Have {
            key: "Kanai2026".into(),
            starred: true
        }
    );
    let like = standing(
        &mut shelf,
        &paper("Network biology predictions from transfer learning of single cells"),
    )
    .unwrap();
    assert_eq!(
        like,
        Standing::Like("Transfer learning enables predictions in network biology".into())
    );
    let new = standing(&mut shelf, &paper("A census of deep-sea octopus behaviour")).unwrap();
    assert_eq!(new, Standing::New);
}

#[test]
fn paperstack_is_asked_over_mcp_one_line_a_message() {
    let replies = [
        r#"{"jsonrpc":"2.0","id":1,"result":{"protocolVersion":"2025-06-18","capabilities":{},"serverInfo":{"name":"paperstack"}}}"#,
        r#"{"jsonrpc":"2.0","id":2,"result":{"content":[{"type":"text","text":"..."}],"isError":false,"structuredContent":{"total":1,"shown":1,"papers":[{"key":"Kanai2026","title":"Population-scale atlas","starred":true}]}}}"#,
        r#"{"jsonrpc":"2.0","id":3,"result":{"content":[{"type":"text","text":"added"}],"isError":false,"structuredContent":{"outcome":"added","paper":{"key":"New2026","title":"A new one","starred":false}}}}"#,
        r#"{"jsonrpc":"2.0","id":4,"result":{"content":[{"type":"text","text":"not found: nothing"}],"isError":true}}"#,
    ]
    .join("\n");
    let mut mcp =
        crate::paperstack::Mcp::new(std::io::Cursor::new(replies), Vec::<u8>::new()).unwrap();
    use crate::paperstack::Library;
    let found = mcp.search("is:starred atlas", 5).unwrap();
    assert_eq!(found[0].key, "Kanai2026");
    assert!(found[0].starred);
    let added = mcp.add("10.1056/AIoa2500123").unwrap();
    assert_eq!(added.key, "New2026");
    let failed = mcp.star("nothing").unwrap_err();
    assert!(format!("{failed:#}").contains("not found"), "{failed:#}");
    // What was asked: initialize, the initialized notice, then each tool.
    let asked: Vec<serde_json::Value> = String::from_utf8(mcp.into_writer())
        .unwrap()
        .lines()
        .map(|l| serde_json::from_str(l).unwrap())
        .collect();
    assert_eq!(asked[0]["method"], "initialize");
    assert_eq!(asked[1]["method"], "notifications/initialized");
    assert_eq!(asked[2]["params"]["name"], "search_papers");
    assert_eq!(asked[2]["params"]["arguments"]["query"], "is:starred atlas");
    assert_eq!(asked[3]["params"]["name"], "add_paper");
    assert_eq!(asked[4]["params"]["name"], "update_paper");
    assert_eq!(asked[4]["params"]["arguments"]["starred"], true);
}

/// A library shared with the test that put it there.
pub(super) struct Shared(pub(super) Arc<Mutex<Shelf>>);

impl crate::paperstack::Library for Shared {
    fn search(&mut self, q: &str, n: usize) -> anyhow::Result<Vec<crate::paperstack::Found>> {
        self.0.lock().unwrap().search(q, n)
    }
    fn add(&mut self, what: &str) -> anyhow::Result<crate::paperstack::Found> {
        self.0.lock().unwrap().add(what)
    }
    fn star(&mut self, key: &str) -> anyhow::Result<()> {
        self.0.lock().unwrap().star(key)
    }
}

/// The demo, reading its Scholar alert.
pub(super) fn reading_the_alert() -> App {
    let mut app = demo();
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
    let at = app
        .shown
        .iter()
        .position(|l| l.summary.from == "Google Scholar Alerts")
        .expect("the alert in Archive");
    app.list.select(Some(at));
    app.focus = app::Focus::List;
    press(&mut app, &["<enter>"]);
    app.settle(Duration::from_secs(5));
    assert!(app.message().is_some(), "{}", app.status);
    app
}

#[test]
fn p_lists_the_papers_of_the_message_read_and_adds_one_to_paperstack_starred() {
    use crate::paperstack::{Found, Standing};
    // Nothing read: nothing to list.
    let mut app = demo();
    press(&mut app, &["P"]);
    assert!(!app.dialog.is_open());
    assert!(app.status.contains("reads"), "{}", app.status);
    let mut app = reading_the_alert();
    let shelf = Arc::new(Mutex::new(Shelf {
        papers: vec![Found {
            key: "Kanai2026".into(),
            title: "Population-scale immune multiome atlas reveals regulatory disease mechanisms"
                .into(),
            starred: true,
        }],
        ..Default::default()
    }));
    *app.library.lock().unwrap() = Some(Box::new(Shared(shelf.clone())));
    press(&mut app, &["P"]);
    app.settle(Duration::from_secs(5));
    let app::Dialog::Papers(p) = &app.dialog else {
        panic!("no papers: {}", app.status);
    };
    assert_eq!(p.papers.len(), 2);
    assert_eq!(
        p.standing[0],
        Some(Standing::Have {
            key: "Kanai2026".into(),
            starred: true
        })
    );
    assert_eq!(p.standing[1], Some(Standing::New));
    let screen = app::screen(&mut app, 120, 40);
    assert!(screen.contains("in your library"), "{screen}");
    assert!(screen.contains("Learning gene programs"), "{screen}");
    // o opens where it is.
    press(&mut app, &["j", "o"]);
    assert_eq!(
        app.launch.take(),
        Some(app::Launch::Browse(
            "https://arxiv.org/abs/2410.01234".into()
        ))
    );
    // ⏎ on the new one: added, and starred, for you liked it; the list
    // put away, and what was done said below.
    press(&mut app, &["<enter>"]);
    app.settle(Duration::from_secs(5));
    {
        let shelf = shelf.lock().unwrap();
        assert_eq!(shelf.added, ["arXiv:2410.01234"]);
        assert!(shelf.papers.iter().all(|p| p.starred));
    }
    assert!(!app.dialog.is_open(), "still open");
    assert!(app.status.contains("in paperstack"), "{}", app.status);
}

#[test]
fn without_paperstack_p_says_how_to_have_it() {
    let mut app = reading_the_alert();
    // SAFETY: this test alone reads it.
    unsafe { std::env::set_var("LETTERBOX_PAPERSTACK", "no-such-paperstack-program") };
    press(&mut app, &["P"]);
    app.settle(Duration::from_secs(5));
    unsafe { std::env::remove_var("LETTERBOX_PAPERSTACK") };
    let app::Dialog::Papers(p) = &app.dialog else {
        panic!("no papers: {}", app.status);
    };
    // The papers are still listed; why paperstack could not be asked, said.
    assert_eq!(p.papers.len(), 2);
    let note = p.note.clone().unwrap_or_default();
    assert!(note.contains("paperstack"), "{note}");
}

#[test]
fn mail_read_before_the_index_read_as_it_now_does_is_read_again_a_few_a_pass() {
    let mut store = Demo::new();
    let shared = std::sync::Mutex::new(Index::memory());
    let stop = std::sync::atomic::AtomicBool::new(false);
    let now = jiff::Timestamp::now().as_second();
    index::sync::pass(&mut store, &shared, now, &stop, &mut |_| {}).unwrap();
    // Read as fresh mail is.
    assert!(
        shared
            .lock()
            .unwrap()
            .records
            .values()
            .all(|r| r.read_as == index::record::READ_AS)
    );
    // Every message read as an older letterbox read it.
    let stale: Vec<index::Record> = shared
        .lock()
        .unwrap()
        .records
        .values()
        .cloned()
        .map(|mut r| {
            r.read_as = 0;
            r.bulk = false;
            r
        })
        .collect();
    shared
        .lock()
        .unwrap()
        .cache(
            stale
                .iter()
                .cloned()
                .map(|r| Cached::Upsert(Box::new(r)))
                .collect(),
        )
        .unwrap();
    let old = |index: &Index| {
        index
            .records
            .values()
            .filter(|r| r.read_as < index::record::READ_AS)
            .count()
    };
    let n = stale.len();
    assert!(n > index::sync::AGAIN, "{n}");
    // A pass reads so many again, the Inbox first; the next, the rest.
    index::sync::pass(&mut store, &shared, now, &stop, &mut |_| {}).unwrap();
    {
        let index = shared.lock().unwrap();
        assert_eq!(old(&index), n - index::sync::AGAIN);
        assert!(
            index
                .records
                .values()
                .filter(|r| index.system(&r.key).inbox)
                .all(|r| r.read_as == index::record::READ_AS),
            "the Inbox first"
        );
        assert!(
            index.records.values().any(|r| r.bulk),
            "the robots' mail, bulk again"
        );
    }
    for _ in 0..n / index::sync::AGAIN + 1 {
        index::sync::pass(&mut store, &shared, now, &stop, &mut |_| {}).unwrap();
    }
    assert_eq!(old(&shared.lock().unwrap()), 0);
}

// ---- marking in the list ----

#[test]
fn in_the_list_what_is_selected_with_the_mouse_is_acted_on_together() {
    use super::selecting::{ctrl_click, select_rows};
    let mut app = demo();
    let first = app.shown[0].summary.subject.clone();
    let second = app.shown[1].summary.subject.clone();
    // Dragged over, three; Ctrl and a click on one, it let go.
    select_rows(&mut app, 0, 2);
    assert_eq!(app.map.region.as_ref().map(|r| r.keys.len()), Some(3));
    ctrl_click(&mut app, 2);
    assert_eq!(app.map.region.as_ref().map(|r| r.keys.len()), Some(2));
    let screen = app::screen(&mut app, 150, 30);
    assert!(screen.contains("2 marked"), "{screen}");
    assert!(screen.contains('▪'), "marked rows marked: {screen}");
    // u marks them read, together.
    press(&mut app, &["u"]);
    app.settle(Duration::from_secs(5));
    for subject in [&first, &second] {
        let l = app
            .shown
            .iter()
            .find(|l| &l.summary.subject == subject)
            .unwrap();
        assert!(l.summary.flags.seen, "{subject} read");
    }
    // m drops down the labels; ⏎ asks, and y files both there together.
    press(&mut app, &["m"]);
    assert!(app.menu.is_some());
    let label = match &app.menu_items()[0] {
        app::Pick::Label(n, _) => n.clone(),
        other => panic!("{other:?}"),
    };
    press(&mut app, &["<enter>"]);
    assert!(
        matches!(app.dialog, app::Dialog::Bulk(_)),
        "asked: {}",
        app.status
    );
    press(&mut app, &["y"]);
    app.settle(Duration::from_secs(5));
    for subject in [&first, &second] {
        let (labels, inbox) = dot_labels(&app, subject);
        assert!(labels.contains(&label) && !inbox, "{subject}: {labels:?}");
    }
    assert!(app.map.region.is_none(), "let go once filed");
    // Marked again, esc lets go; d trashes what is marked.
    ctrl_click(&mut app, 0);
    press(&mut app, &["<esc>"]);
    assert!(app.map.region.is_none());
    let count = app.shown.len();
    // Inbox mail, marked: asked first, and taken only with i.
    select_rows(&mut app, 0, 1);
    press(&mut app, &["d"]);
    assert!(matches!(app.dialog, app::Dialog::Bulk(_)));
    press(&mut app, &["i", "y"]);
    app.settle(Duration::from_secs(5));
    assert_eq!(app.shown.len(), count - 2);
}

#[test]
fn in_the_list_what_the_mouse_selects_is_acted_on_together_and_no_key_selects() {
    let mut app = demo();
    let marked = |app: &App| app.map.region.as_ref().map_or(0, |r| r.keys.len());
    // No key selects: neither space, nor Shift and an arrow.
    press(&mut app, &[" ", "g", "<s-down>", "<s-down>", "<s-up>"]);
    assert_eq!(marked(&app), 0);
    // Rows dragged over, with the mouse.
    super::selecting::select_rows(&mut app, 0, 2);
    assert_eq!(marked(&app), 3);
    let screen = app::screen(&mut app, 150, 30);
    assert!(screen.contains("3 marked"), "{screen}");
    // Then a key acts on all of them, once asked: d, i and y trash the
    // three, Inbox mail as they are.
    let count = app.shown.len();
    press(&mut app, &["d", "i", "y"]);
    app.settle(Duration::from_secs(5));
    assert_eq!(app.shown.len(), count - 3);
}

#[test]
fn in_the_inbox_looked_through_the_mouse_selects_and_a_choice_is_for_all() {
    use super::selecting::drag;
    use app::Target::ReviewRow;
    let mut app = demo_map();
    press(&mut app, &["i"]);
    let rows = app.review_rows();
    let selected = |app: &App| app.review.as_ref().unwrap().selected.len();
    // Shift and an arrow select nothing.
    press(&mut app, &["<s-down>", "<s-down>"]);
    assert_eq!(selected(&app), 0);
    press(&mut app, &["g"]);
    drag(&mut app, ReviewRow(0), ReviewRow(2));
    assert_eq!(selected(&app), 3);
    let screen = app::screen(&mut app, 170, 40);
    assert!(screen.contains("3 selected"), "{screen}");
    // d discards the three, and goes on past them.
    press(&mut app, &["d"]);
    for &i in &rows[..3] {
        assert_eq!(app.choice(i), app::Choice::Discard);
    }
    assert_eq!(selected(&app), 0);
    let picked = app.review_at().map(|(i, _)| i);
    assert_eq!(picked, Some(rows[3]));
    // Space keeps the two selected.
    drag(&mut app, ReviewRow(3), ReviewRow(4));
    press(&mut app, &[" "]);
    assert_eq!(app.choice(rows[3]), app::Choice::Keep);
    assert_eq!(app.choice(rows[4]), app::Choice::Keep);
    // m: the label picked for both.
    drag(&mut app, ReviewRow(5), ReviewRow(6));
    press(&mut app, &["m"]);
    let label = match &app.menu_items()[0] {
        app::Pick::Label(n, _) => n.clone(),
        other => panic!("{other:?}"),
    };
    press(&mut app, &["<enter>"]);
    for &i in &rows[5..7] {
        assert_eq!(app.choice(i), app::Choice::Move(label.clone()));
    }
    // esc lets the selection go first, and only then goes back to the map.
    drag(&mut app, ReviewRow(7), ReviewRow(8));
    press(&mut app, &["<esc>"]);
    assert_eq!(selected(&app), 0);
    assert_eq!(app.focus, app::Focus::Reader);
}

#[test]
fn in_the_list_a_message_filed_is_out_of_it_at_once_and_the_next_picked() {
    let mut app = demo();
    let at = app.list.selected().unwrap();
    let subject = app.shown[at].summary.subject.clone();
    let uid = app.shown[at].summary.uid;
    let next = app.shown[at + 1].summary.subject.clone();
    press(&mut app, &["<enter>"]);
    app.settle(Duration::from_secs(5));
    assert!(app.reading.is_some());
    // Filed while read: out of the list, and the next picked, before the
    // server has said so.
    press(&mut app, &["m", "<enter>"]);
    assert!(
        !app.shown.iter().any(|l| l.summary.subject == subject),
        "still listed"
    );
    assert_eq!(app.current().unwrap().summary.subject, next);
    assert!(
        app.reading.as_ref().is_none_or(|r| r.uid != uid),
        "still read"
    );
    // And once it has, still out of it.
    app.settle(Duration::from_secs(5));
    assert!(!app.shown.iter().any(|l| l.summary.subject == subject));
}

#[test]
fn in_the_list_y_capital_files_all_like_the_one_picked_once_asked() {
    let mut app = demo();
    let s = app.map.snapshot.clone().unwrap();
    let going = app
        .shown
        .iter()
        .position(|l| {
            s.dots
                .iter()
                .enumerate()
                .any(|(i, d)| d.subject == l.summary.subject && s.suggestion(i).is_some())
        })
        .unwrap();
    let subject = app.shown[going].summary.subject.clone();
    let i = s.dots.iter().position(|d| d.subject == subject).unwrap();
    let label = s.suggestion(i).unwrap().to_string();
    let like: Vec<String> = app
        .shown
        .iter()
        .filter(|l| {
            s.dots.iter().enumerate().any(|(j, d)| {
                d.subject == l.summary.subject && s.suggestion(j) == Some(label.as_str())
            })
        })
        .map(|l| l.summary.subject.clone())
        .collect();
    app.list.select(Some(going));
    // All like it, asked first, counted; y files them.
    press(&mut app, &["Y"]);
    let app::Dialog::Bulk(bulk) = &app.dialog else {
        panic!("asked: {}", app.status);
    };
    assert_eq!(bulk.keys.len(), like.len());
    assert!(bulk.said.title.contains(&label), "{}", bulk.said.title);
    press(&mut app, &["y"]);
    app.settle(Duration::from_secs(5));
    for subject in &like {
        let (labels, inbox) = dot_labels(&app, subject);
        assert!(labels.contains(&label) && !inbox, "{subject}: {labels:?}");
    }
}

// ---- threads, by who answered whom ----

pub(super) fn threaded(id: &str, parents: &[&str], subject: &str) -> crate::forest::Threaded {
    crate::forest::Threaded {
        message_id: id.to_string(),
        parents: parents.iter().map(|p| p.to_string()).collect(),
        thread: None,
        subject: subject.to_string(),
    }
}

pub(super) fn sorted(mut groups: Vec<Vec<usize>>) -> Vec<Vec<usize>> {
    for g in &mut groups {
        g.sort();
    }
    groups.sort();
    groups
}

#[test]
fn a_thread_is_who_answered_whom_not_its_subject() {
    let items = vec![
        threaded("a", &[], "Meeting"),
        threaded("b", &["a"], "Re: Meeting"),
        // Its subject changed: still the same talk.
        threaded("c", &["a", "b"], "Moved to Thursday"),
        // Another talk of the same subject: apart.
        threaded("d", &[], "Meeting"),
        threaded("e", &["d"], "Re: Meeting"),
        // Notices that answer nothing: by subject, as before.
        threaded("f", &[], "Build failed: 12"),
        threaded("g", &[], "Build failed: 13"),
        // Answers to one not here, together.
        threaded("h", &["lost"], "Re: lost"),
        threaded("i", &["lost"], "Re: lost, again"),
    ];
    assert_eq!(
        sorted(crate::forest::thread_groups(&items)),
        vec![vec![0, 1, 2], vec![3, 4], vec![5, 6], vec![7, 8]]
    );
}

#[test]
fn gmails_thread_holds_its_messages_together() {
    let mut a = threaded("a", &[], "Photo 51");
    let mut b = threaded("b", &[], "the X-ray, sharper");
    a.thread = Some(9);
    b.thread = Some(9);
    let c = threaded("c", &[], "Something else");
    assert_eq!(
        sorted(crate::forest::thread_groups(&[a, b, c])),
        vec![vec![0, 1], vec![2]]
    );
}

#[test]
fn what_a_message_answers_is_read_from_its_headers() {
    let r = with_headers(
        "x",
        "ada@analytical.example",
        "Re: Notes",
        "In-Reply-To: <b@analytical.example>\r\nReferences: <a@analytical.example>\r\n <b@analytical.example>\r\n",
    );
    assert_eq!(r.parents, ["b@analytical.example", "a@analytical.example"]);
    assert_eq!(r.read_as, 2, "read again from older mail");
    let items = crate::imap::indexed_items(false);
    assert!(items.contains("IN-REPLY-TO") && items.contains("REFERENCES"));
}

#[test]
fn the_message_read_is_let_go_when_the_list_gives_way_to_the_map_and_back() {
    // Read in the list: it stays while the list is gone along.
    let mut app = demo();
    app.list.select(Some(0));
    press(&mut app, &["<enter>"]);
    let _ = app::screen(&mut app, 150, 45);
    assert!(app.reading.is_some(), "read");
    press(&mut app, &["<down>", "<up>"]);
    let _ = app::screen(&mut app, 150, 45);
    assert!(app.reading.is_some(), "still read, in the list it is in");
    // To the map: the list's message is not beside it.
    press(&mut app, &["M"]);
    let screen = app::screen(&mut app, 150, 45);
    assert!(app.reading.is_none(), "{screen}");
    // Read on the map, and back to the list: not beside the list either.
    read_on_map(&mut app);
    assert!(app.reading.is_some(), "read on the map");
    press(&mut app, &["M"]);
    let screen = app::screen(&mut app, 150, 45);
    assert!(app.reading.is_none(), "{screen}");
    assert!(screen.contains("reads the message picked"), "{screen}");
}
